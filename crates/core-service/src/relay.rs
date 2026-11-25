use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use shared::{LogCategory, LogLevel, RuleAction};
use tokio::{
    io::{self as aio, AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

use crate::{packet::FlowKey, process::Decision, sniffer, socks5, telemetry::Telemetry};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_LINGER: Duration = Duration::from_secs(15);
const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub(crate) struct NatKey {
    local: SocketAddrV4,
    remote_ip: Ipv4Addr,
}

impl NatKey {
    pub fn new(local: SocketAddrV4, remote_ip: Ipv4Addr) -> Self {
        Self { local, remote_ip }
    }

    pub fn from_flow(key: FlowKey) -> Self {
        Self::new(key.local, *key.remote.ip())
    }

    pub fn from_relay_reply(key: FlowKey) -> Self {
        // The outgoing relay reply runs from app_ip:relay_port to remote_ip:app_port.
        Self::new(
            SocketAddrV4::new(*key.local.ip(), key.remote.port()),
            *key.remote.ip(),
        )
    }
}

#[derive(Clone)]
pub(crate) struct Mapping {
    pub flow: FlowKey,
    pub id: u64,
    pub decision: Decision,
    pub interface: [u32; 2],
}

struct NatEntry {
    mapping: Mapping,
    expires: Option<Instant>,
    claimed: bool,
}

#[derive(Default)]
pub(crate) struct NatTable(Mutex<HashMap<NatKey, NatEntry>>);

impl NatTable {
    pub fn insert(&self, key: NatKey, mapping: Mapping) -> bool {
        let entry = NatEntry {
            mapping,
            expires: Some(Instant::now() + ACCEPT_TIMEOUT),
            claimed: false,
        };
        match self.0.lock().unwrap().entry(key) {
            Entry::Occupied(mut slot) if slot.get().claimed && slot.get().expires.is_some() => {
                slot.insert(entry);
                true
            }
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(entry);
                true
            }
        }
    }

    pub fn find(&self, key: NatKey) -> Option<Mapping> {
        self.0
            .lock()
            .unwrap()
            .get(&key)
            .map(|entry| entry.mapping.clone())
    }

    fn claim(&self, key: NatKey) -> Option<Mapping> {
        let mut mappings = self.0.lock().unwrap();
        let entry = mappings.get_mut(&key)?;
        if entry.claimed || entry.expires.is_some_and(|expiry| expiry <= Instant::now()) {
            return None;
        }
        entry.claimed = true;
        entry.expires = None;
        Some(entry.mapping.clone())
    }

    fn close(&self, key: NatKey, id: u64) {
        let mut mappings = self.0.lock().unwrap();
        if let Some(entry) = mappings
            .get_mut(&key)
            .filter(|entry| entry.mapping.id == id)
        {
            entry.expires = Some(Instant::now() + CLOSE_LINGER);
        }
    }

    pub fn remove_unclaimed(&self, flow: FlowKey, id: u64) -> bool {
        let key = NatKey::from_flow(flow);
        let mut mappings = self.0.lock().unwrap();
        if mappings
            .get(&key)
            .is_some_and(|entry| entry.mapping.id == id && !entry.claimed)
        {
            mappings.remove(&key);
            true
        } else {
            false
        }
    }

    // Return only unaccepted handshakes. Connected streams finish their own telemetry.
    pub fn expire(&self) -> Vec<Mapping> {
        let now = Instant::now();
        let mut expired = Vec::new();
        self.0.lock().unwrap().retain(|_, entry| {
            let keep = entry.expires.is_none_or(|deadline| deadline > now);
            if !keep && !entry.claimed {
                expired.push(entry.mapping.clone());
            }
            keep
        });
        expired
    }
}

pub(crate) async fn run(
    listener: TcpListener,
    nat: Arc<NatTable>,
    telemetry: Arc<Telemetry>,
) -> io::Result<()> {
    let mut streams = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (client, peer) = accepted?;
                let _ = client.set_nodelay(true);
                let (SocketAddr::V4(local), SocketAddr::V4(peer)) = (client.local_addr()?, peer) else { continue };
                let key = NatKey::new(SocketAddrV4::new(*local.ip(), peer.port()), *peer.ip());
                let Some(mapping) = nat.claim(key) else { continue };
                let nat = Arc::clone(&nat);
                let telemetry = Arc::clone(&telemetry);
                streams.spawn(async move {
                    transfer(client, &mapping, &telemetry).await;
                    nat.close(key, mapping.id);
                });
            }
            Some(result) = streams.join_next(), if !streams.is_empty() => {
                if let Err(error) = result {
                    tracing::error!(%error, "relay task failed");
                }
            }
        }
    }
}

async fn transfer(client: TcpStream, mapping: &Mapping, telemetry: &Telemetry) {
    let domain = sniffer::peek_domain(&client).await;
    if let Some(domain) = &domain {
        telemetry.domain(mapping.id, domain.clone());
    }
    let mut action = mapping.decision.action;
    if let Some(hosts) = &mapping.decision.pending_hosts {
        let matched = hosts.matches(*mapping.flow.remote.ip(), domain.as_deref());
        if !matched {
            action = RuleAction::Direct;
        }
        telemetry.resolve_route(mapping.id, action, matched);
    }
    if action == RuleAction::Block {
        telemetry.finish(mapping.id, Some("Blocked by routing policy".into()));
        return;
    }

    let fail = |title, error: io::Error| {
        telemetry.log(
            LogLevel::Warn,
            LogCategory::Proxy,
            title,
            Some(format!("{}: {error}", mapping.flow.remote)),
        );
        telemetry.finish(mapping.id, Some(error.to_string()));
    };
    let upstream = match connect_upstream(mapping, telemetry, action, domain.as_deref()).await {
        Ok(upstream) => upstream,
        Err(error) => {
            let title = match action {
                RuleAction::Proxy if mapping.decision.network_lock => {
                    "Locked proxy connection failed"
                }
                RuleAction::Proxy => "Proxy connection failed",
                _ => "Direct relay connection failed",
            };
            fail(title, error);
            return;
        }
    };
    telemetry.activate(mapping.id);
    let mut client = Counted {
        stream: client,
        telemetry,
        id: mapping.id,
        is_upstream: false,
    };
    let mut upstream = Counted {
        stream: upstream,
        telemetry,
        id: mapping.id,
        is_upstream: true,
    };
    match aio::copy_bidirectional(&mut client, &mut upstream).await {
        Err(error) if !is_graceful_teardown(&error) => fail("Relay I/O failed", error),
        _ => telemetry.finish(mapping.id, None),
    }
}

fn is_graceful_teardown(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof
    ) || matches!(error.raw_os_error(), Some(10053 | 10054 | 10058))
}

async fn connect_upstream(
    mapping: &Mapping,
    telemetry: &Telemetry,
    action: RuleAction,
    domain: Option<&str>,
) -> io::Result<TcpStream> {
    let remote = mapping.flow.remote;
    if action == RuleAction::Proxy {
        let proxy = mapping.decision.proxy.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no enabled upstream proxy is configured",
            )
        })?;
        match socks5::connect(proxy, remote, domain).await {
            Ok(stream) => return Ok(stream),
            Err(error) if !mapping.decision.network_lock && proxy.fallback_direct => {
                tracing::warn!(destination = %remote, %error, "SOCKS5 connect failed; retrying directly");
                telemetry.log(
                    LogLevel::Warn,
                    LogCategory::Proxy,
                    "SOCKS5 connect failed; retrying directly",
                    Some(format!("{remote}: {error}")),
                );
                telemetry.resolve_route(mapping.id, RuleAction::Direct, true);
            }
            Err(error) => return Err(error),
        }
    }
    tokio::time::timeout(DIRECT_CONNECT_TIMEOUT, TcpStream::connect(remote))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Direct connection timed out"))?
}

struct Counted<'a> {
    stream: TcpStream,
    telemetry: &'a Telemetry,
    id: u64,
    is_upstream: bool,
}

impl AsyncRead for Counted<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for Counted<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = result {
            let (up, down) = if self.is_upstream {
                (count, 0)
            } else {
                (0, count)
            };
            self.telemetry.bytes(self.id, up, down);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{ConnectionState, ProxyAuth, ProxyConfig, ProxyProtocol};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::timeout,
    };

    #[test]
    fn graceful_teardown_matches_socket_shutdown_errors() {
        use io::ErrorKind::*;

        for kind in [
            ConnectionReset,
            ConnectionAborted,
            BrokenPipe,
            NotConnected,
            UnexpectedEof,
        ] {
            assert!(is_graceful_teardown(&io::Error::from(kind)), "{kind:?}");
        }
        for code in [10053, 10054, 10058] {
            assert!(
                is_graceful_teardown(&io::Error::from_raw_os_error(code)),
                "{code}"
            );
        }
        for kind in [TimedOut, ConnectionRefused] {
            assert!(!is_graceful_teardown(&io::Error::from(kind)), "{kind:?}");
        }
    }

    #[tokio::test]
    async fn established_proxy_transfer_closes_without_warning_after_client_resets() {
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(relay.local_addr().unwrap())
            .await
            .unwrap();
        let (accepted, _) = relay.accept().await.unwrap();
        let flow = FlowKey {
            local: client.local_addr().unwrap().to_string().parse().unwrap(),
            remote: "203.0.113.1:443".parse().unwrap(),
        };
        let decision = Decision {
            action: RuleAction::Proxy,
            proxy: Some(ProxyConfig {
                id: "mock".into(),
                name: "Mock".into(),
                protocol: ProxyProtocol::Socks5,
                host: "127.0.0.1".into(),
                port: proxy.local_addr().unwrap().port(),
                auth: None,
                interface_name: None,
                allow_lan: false,
                udp_relay: false,
                fallback_direct: false,
                enabled: true,
            }),
            ..Decision::direct()
        };
        let telemetry = Telemetry::new();
        let id = telemetry.open(flow, &crate::process::Process::unknown(1), &decision, true);
        let mapping = Mapping {
            flow,
            id,
            decision,
            interface: [0, 0],
        };

        let proxy_side = async {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&[5, 0]).await.unwrap();
            let mut connect = [0; 10];
            socket.read_exact(&mut connect).await.unwrap();
            assert_eq!(connect, [5, 1, 0, 1, 203, 0, 113, 1, 1, 187]);
            socket
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();

            let mut request = [0; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            socket.write_all(b"pong").await.unwrap();
            // Keep the proxy open so the client reset is what ends the relay.
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        };
        let client_side = async move {
            let mut client = client;
            client.write_all(b"ping").await.unwrap();
            let mut response = [0; 4];
            client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"pong");
            client.set_zero_linger().unwrap();
            drop(client);
        };

        timeout(Duration::from_secs(3), async {
            tokio::join!(
                proxy_side,
                client_side,
                transfer(accepted, &mapping, &telemetry)
            )
        })
        .await
        .unwrap();

        let snapshot = telemetry.connections(true, Some(1)).remove(0);
        assert_eq!(snapshot.state, ConnectionState::Closed);
        assert!(snapshot.error.is_none());
        assert_eq!((snapshot.bytes_up, snapshot.bytes_down), (4, 4));
        assert!(telemetry.logs(None).is_empty());
    }

    #[test]
    fn remove_unclaimed_checks_id_and_preserves_claimed_mappings() {
        let nat = NatTable::default();
        let flow = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        let key = NatKey::from_flow(flow);
        let mapping = Mapping {
            flow,
            id: 1,
            decision: Decision::direct(),
            interface: [0, 0],
        };
        assert!(!nat.remove_unclaimed(flow, 1));
        assert!(nat.insert(key, mapping.clone()));
        assert!(!nat.remove_unclaimed(flow, 2));
        assert!(nat.remove_unclaimed(flow, 1));
        assert!(nat.find(key).is_none());

        assert!(nat.insert(key, mapping));
        assert_eq!(nat.claim(key).unwrap().id, 1);
        assert!(!nat.remove_unclaimed(flow, 1));
        assert_eq!(nat.find(key).unwrap().id, 1);
        nat.close(key, 1);
        assert!(!nat.remove_unclaimed(flow, 1));
        // A closed relay keeps the mapping for late TCP replies and expires it normally.
        nat.0.lock().unwrap().get_mut(&key).unwrap().expires = Some(Instant::now());
        assert!(nat.expire().is_empty());
        assert!(nat.find(key).is_none());
    }

    #[test]
    fn insert_replaces_lingering_nat_entry_after_relay_closes() {
        let nat = NatTable::default();
        let flow = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        let key = NatKey::from_flow(flow);
        let original = Mapping {
            flow,
            id: 1,
            decision: Decision::direct(),
            interface: [0, 0],
        };
        let replacement = Mapping {
            id: 2,
            ..original.clone()
        };
        assert!(nat.insert(key, original));
        assert!(!nat.insert(key, replacement.clone()));
        assert_eq!(nat.claim(key).unwrap().id, 1);
        assert!(!nat.insert(key, replacement.clone()));

        nat.close(key, 1);
        assert!(nat.insert(key, replacement));
        assert_eq!(nat.claim(key).unwrap().id, 2);
        nat.close(key, 1);
        assert!(nat.0.lock().unwrap().get(&key).unwrap().expires.is_none());
    }

    #[test]
    fn nat_lookup_matches_original_flow_and_relay_reply() {
        let key = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        let reply = FlowKey {
            local: "192.0.2.1:61000".parse().unwrap(),
            remote: "203.0.113.2:55000".parse().unwrap(),
        };
        assert_eq!(NatKey::from_flow(key), NatKey::from_relay_reply(reply));
    }

    #[tokio::test]
    async fn proxy_failure_retries_directly_only_when_enabled_and_unlocked() {
        for (fallback_direct, network_lock) in
            [(false, false), (false, true), (true, true), (true, false)]
        {
            let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let flow = FlowKey {
                local: "127.0.0.1:50000".parse().unwrap(),
                remote: destination
                    .local_addr()
                    .unwrap()
                    .to_string()
                    .parse()
                    .unwrap(),
            };
            let decision = Decision {
                action: RuleAction::Proxy,
                network_lock,
                proxy: Some(ProxyConfig {
                    id: "rejecting".into(),
                    name: "Rejecting proxy".into(),
                    protocol: ProxyProtocol::Socks5,
                    host: "127.0.0.1".into(),
                    port: proxy.local_addr().unwrap().port(),
                    auth: None,
                    interface_name: None,
                    allow_lan: false,
                    udp_relay: false,
                    fallback_direct,
                    enabled: true,
                }),
                ..Decision::direct()
            };
            let telemetry = Telemetry::new();
            let id = telemetry.open(flow, &crate::process::Process::unknown(1), &decision, true);
            let mapping = Mapping {
                flow,
                id,
                decision,
                interface: [0, 0],
            };
            let reject = async {
                let (mut socket, _) = proxy.accept().await.unwrap();
                let mut greeting = [0; 3];
                socket.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                socket.write_all(&[5, 0]).await.unwrap();
                let mut request = [0; 10];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..4], &[5, 1, 0, 1]);
                socket
                    .write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
            };
            let connect = connect_upstream(&mapping, &telemetry, RuleAction::Proxy, None);
            let ((), result) = timeout(Duration::from_secs(2), async {
                tokio::join!(reject, connect)
            })
            .await
            .unwrap();
            let snapshot = telemetry.connections(false, Some(1)).remove(0);
            if fallback_direct && !network_lock {
                assert_eq!(
                    result.unwrap().peer_addr().unwrap(),
                    destination.local_addr().unwrap()
                );
                assert_eq!(snapshot.action, RuleAction::Direct);
                assert!(snapshot.proxy_id.is_none());
                assert!(telemetry
                    .logs(None)
                    .iter()
                    .any(|entry| matches!(entry.level, LogLevel::Warn)));
            } else {
                assert!(result.unwrap_err().to_string().contains("reply 0x05"));
                assert_eq!(snapshot.action, RuleAction::Proxy);
                assert!(telemetry.logs(None).is_empty());
            }
        }
    }

    fn domain_filter(remote: SocketAddrV4) -> Option<Arc<crate::targets::HostPatterns>> {
        let rule = shared::Rule {
            id: "domains".into(),
            profile_id: "default".into(),
            name: "Domains".into(),
            enabled: true,
            matcher: shared::RuleMatcher::ProcessName {
                name: "curl.exe".into(),
                include_children: false,
            },
            action: RuleAction::Proxy,
            proxy_id: None,
            network_lock: true,
            priority: 0,
            target_hosts: vec!["*.example.org".into(), "-ads.example.org".into()],
            target_ports: Vec::new(),
        };
        crate::targets::Targets::parse(&rule)
            .unwrap()
            .check(*remote.ip(), remote.port())
            .unwrap()
    }

    #[tokio::test]
    async fn relay_dials_original_ip_for_unmatched_and_excluded_domains() {
        for (host, action) in [
            ("other.test", RuleAction::Proxy),
            ("ads.example.org", RuleAction::Proxy),
            ("ads.example.org", RuleAction::Block),
        ] {
            let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = TcpStream::connect(relay.local_addr().unwrap())
                .await
                .unwrap();
            let (accepted, _) = relay.accept().await.unwrap();
            let flow = FlowKey {
                local: client.local_addr().unwrap().to_string().parse().unwrap(),
                remote: destination
                    .local_addr()
                    .unwrap()
                    .to_string()
                    .parse()
                    .unwrap(),
            };
            let decision = Decision {
                action,
                pending_hosts: domain_filter(flow.remote),
                network_lock: true,
                rule_id: Some("domains".into()),
                ..Decision::direct()
            };
            let telemetry = Telemetry::new();
            let id = telemetry.open(flow, &crate::process::Process::unknown(1), &decision, true);
            let mapping = Mapping {
                flow,
                id,
                decision,
                interface: [0, 0],
            };
            let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
            let response = b"HTTP/1.1 204 No Content\r\n\r\n";
            let server = async {
                let (mut socket, _) = destination.accept().await.unwrap();
                let mut bytes = vec![0; request.len()];
                socket.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, request.as_bytes());
                socket.write_all(response).await.unwrap();
                socket.shutdown().await.unwrap();
            };
            let client = async {
                client.write_all(request.as_bytes()).await.unwrap();
                client.shutdown().await.unwrap();
                let mut bytes = Vec::new();
                client.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, response);
            };
            timeout(Duration::from_secs(2), async {
                tokio::join!(server, client, transfer(accepted, &mapping, &telemetry))
            })
            .await
            .unwrap();
            let snapshot = telemetry.connections(true, Some(1)).remove(0);
            assert_eq!(snapshot.domain.as_deref(), Some(host));
            assert_eq!(snapshot.action, RuleAction::Direct);
            assert_eq!(snapshot.state, ConnectionState::Closed);
            assert!(snapshot.rule_id.is_none());
            assert_eq!(
                (snapshot.bytes_up, snapshot.bytes_down),
                (request.len() as u64, response.len() as u64)
            );
        }
    }

    #[tokio::test]
    async fn loopback_relay_peeks_domain_authenticates_and_copies_both_ways() {
        let relay = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = (Ipv4Addr::LOCALHOST, relay.local_addr().unwrap().port());
        let mut client = TcpStream::connect(relay_addr).await.unwrap();
        let local: SocketAddrV4 = client.local_addr().unwrap().to_string().parse().unwrap();
        let flow = FlowKey {
            local,
            remote: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 443),
        };
        let decision = Decision {
            action: RuleAction::Proxy,
            network_lock: true,
            pending_hosts: domain_filter(flow.remote),
            proxy: Some(ProxyConfig {
                id: "mock".into(),
                name: "Mock".into(),
                protocol: ProxyProtocol::Socks5,
                host: "127.0.0.1".into(),
                port: proxy.local_addr().unwrap().port(),
                auth: Some(ProxyAuth {
                    username: "alice".into(),
                    password: "secret".into(),
                }),
                interface_name: None,
                allow_lan: false,
                udp_relay: false,
                fallback_direct: true,
                enabled: true,
            }),
            ..Decision::direct()
        };
        let telemetry = Telemetry::new();
        let id = telemetry.open(flow, &crate::process::Process::unknown(1), &decision, true);
        let nat = Arc::new(NatTable::default());
        assert!(nat.insert(
            NatKey::from_flow(flow),
            Mapping {
                flow,
                id,
                decision,
                interface: [0, 0]
            }
        ));
        let relay_task = tokio::spawn(run(relay, Arc::clone(&nat), Arc::clone(&telemetry)));
        let request = b"GET / HTTP/1.1\r\nHost: WWW.Example.org\r\n\r\n";
        let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        let proxy_side = async {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 2]);
            socket.write_all(&[5, 2]).await.unwrap();
            let mut credentials = [0; 14];
            socket.read_exact(&mut credentials).await.unwrap();
            assert_eq!(&credentials, b"\x01\x05alice\x06secret");
            socket.write_all(&[1, 0]).await.unwrap();
            let mut header = [0; 5];
            socket.read_exact(&mut header).await.unwrap();
            assert_eq!(header, [5, 1, 0, 3, 15]);
            let mut target = [0; 17];
            socket.read_exact(&mut target).await.unwrap();
            assert_eq!(&target, b"www.example.org\x01\xbb");
            socket
                .write_all(&[5, 0, 0, 3, 1, b'x', 0, 0])
                .await
                .unwrap();
            let mut received = vec![0; request.len()];
            socket.read_exact(&mut received).await.unwrap();
            assert_eq!(received, request);
            socket.write_all(response).await.unwrap();
            socket.shutdown().await.unwrap();
        };
        let client_side = async {
            client.write_all(&request[..9]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            client.write_all(&request[9..]).await.unwrap();
            client.shutdown().await.unwrap();
            let mut received = Vec::new();
            client.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, response);
        };
        timeout(Duration::from_secs(3), async {
            tokio::join!(proxy_side, client_side)
        })
        .await
        .unwrap();
        let snapshot = timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = telemetry.connections(true, Some(1)).remove(0);
                if snapshot.state == ConnectionState::Closed {
                    break snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(snapshot.domain.as_deref(), Some("www.example.org"));
        assert_eq!(
            (snapshot.bytes_up, snapshot.bytes_down),
            (request.len() as u64, response.len() as u64)
        );
        relay_task.abort();
    }
}
