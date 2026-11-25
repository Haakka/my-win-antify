use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use shared::{LogCategory, LogLevel, ProxyConfig, RuleAction};
use tokio::{
    io::{self as aio, AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{oneshot, watch, Notify},
    task::JoinSet,
};

use crate::{
    packet::FlowKey,
    process::Decision,
    sniffer,
    targets::BypassHosts,
    telemetry::Telemetry,
    upstream::{self, UpstreamError, UpstreamStream, CONNECT_TIMEOUT},
};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_LINGER: Duration = Duration::from_secs(15);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);

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
    pub bypass_hosts: Option<Arc<BypassHosts>>,
    pub interface: [u32; 2],
}

struct NatEntry {
    mapping: Mapping,
    expires: Option<Instant>,
    claimed: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
pub(crate) struct NatTable(Mutex<HashMap<NatKey, NatEntry>>);

impl NatTable {
    pub fn insert(&self, key: NatKey, mapping: Mapping) -> bool {
        let entry = NatEntry {
            mapping,
            expires: Some(Instant::now() + ACCEPT_TIMEOUT),
            claimed: None,
        };
        match self.0.lock().unwrap().entry(key) {
            Entry::Occupied(mut slot)
                if slot.get().claimed.is_some() && slot.get().expires.is_some() =>
            {
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

    fn claim(&self, key: NatKey) -> Option<(Mapping, oneshot::Receiver<()>)> {
        let mut mappings = self.0.lock().unwrap();
        let entry = mappings.get_mut(&key)?;
        if entry.claimed.is_some() || entry.expires.is_some_and(|expiry| expiry <= Instant::now()) {
            return None;
        }
        let (retire, retired) = oneshot::channel();
        entry.claimed = Some(retire);
        entry.expires = None;
        Some((entry.mapping.clone(), retired))
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
            .is_some_and(|entry| entry.mapping.id == id && entry.claimed.is_none())
        {
            mappings.remove(&key);
            true
        } else {
            false
        }
    }

    pub fn clear(&self) -> Vec<u64> {
        self.0
            .lock()
            .unwrap()
            .drain()
            .map(|(_, entry)| entry.mapping.id)
            .collect()
    }

    // Return only unaccepted handshakes. Connected streams finish their own telemetry.
    pub fn expire(&self) -> Vec<Mapping> {
        let now = Instant::now();
        let mut expired = Vec::new();
        self.0.lock().unwrap().retain(|_, entry| {
            let keep = entry.expires.is_none_or(|deadline| deadline > now);
            if !keep && entry.claimed.is_none() {
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
    mut cancel: watch::Receiver<bool>,
) -> io::Result<()> {
    let mut streams = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = wait_for_shutdown(&mut cancel) => return Ok(()),
            accepted = listener.accept() => {
                let (client, peer) = accepted?;
                let _ = client.set_nodelay(true);
                let (SocketAddr::V4(local), SocketAddr::V4(peer)) = (client.local_addr()?, peer) else { continue };
                let key = NatKey::new(SocketAddrV4::new(*local.ip(), peer.port()), *peer.ip());
                let Some((mapping, retired)) = nat.claim(key) else { continue };
                let nat = Arc::clone(&nat);
                let telemetry = Arc::clone(&telemetry);
                let mut cancel = cancel.clone();
                streams.spawn(async move {
                    tokio::select! {
                        biased;
                        _ = retired => telemetry.finish(mapping.id, None),
                        () = transfer(client, &mapping, &telemetry, &mut cancel) => {
                            nat.close(key, mapping.id);
                        }
                    }
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

async fn wait_for_shutdown(cancel: &mut watch::Receiver<bool>) {
    let _ = cancel.wait_for(|stopping| *stopping).await;
}

async fn transfer(
    client: TcpStream,
    mapping: &Mapping,
    telemetry: &Telemetry,
    cancel: &mut watch::Receiver<bool>,
) {
    let id = mapping.id;
    let domain = tokio::select! {
        biased;
        () = wait_for_shutdown(cancel) => {
            telemetry.finish(id, None);
            return;
        }
        domain = async {
            if matches!(mapping.flow.remote.port(), 21 | 22 | 25 | 110 | 143 | 587 | 3306)
                && mapping.decision.pending_hosts.is_none() && mapping.bypass_hosts.is_none()
            {
                None
            } else {
                sniffer::peek_domain(&client).await
            }
        } => domain,
    };
    if let Some(domain) = &domain {
        telemetry.domain(id, domain.clone());
    }
    let mut action = mapping.decision.action;
    let remote_ip = *mapping.flow.remote.ip();
    if action == RuleAction::Proxy
        && mapping
            .bypass_hosts
            .as_ref()
            .is_some_and(|hosts| hosts.matches(remote_ip, domain.as_deref()))
    {
        action = RuleAction::Direct;
        let rule_matched = mapping
            .decision
            .pending_hosts
            .as_ref()
            .is_none_or(|hosts| hosts.matches(remote_ip, domain.as_deref()));
        telemetry.resolve_route(id, action, rule_matched);
        telemetry.route_detail(id, Some("Bypassed via bypassHosts"), None);
    } else if let Some(hosts) = &mapping.decision.pending_hosts {
        let matched = hosts.matches(remote_ip, domain.as_deref());
        if !matched {
            action = RuleAction::Direct;
        }
        telemetry.resolve_route(id, action, matched);
        let reason = if matched {
            mapping.decision.match_reason()
        } else {
            "Destination did not match rule".into()
        };
        telemetry.route_detail(id, Some(&reason), None);
    }
    if action == RuleAction::Block {
        telemetry.route_detail(id, None, Some("blocked"));
        telemetry.finish(id, Some("Blocked by routing policy".into()));
        return;
    }

    let fail = |title, error: &UpstreamError| {
        telemetry.log(
            LogLevel::Warn,
            LogCategory::Proxy,
            title,
            Some(format!("{}: {error}", mapping.flow.remote)),
        );
        telemetry.route_detail(id, None, Some(error.code));
        telemetry.finish(id, Some(error.to_string()));
    };
    let generation = telemetry.health_generation();
    let (stream, used_proxy) = match connect_upstream(
        mapping,
        telemetry,
        action,
        domain.as_deref(),
        generation,
        cancel,
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) if error.io.kind() == io::ErrorKind::Interrupted => {
            telemetry.finish(id, None);
            return;
        }
        Err(error) => {
            let title = match action {
                RuleAction::Proxy if mapping.decision.network_lock => {
                    "Locked proxy connection failed"
                }
                RuleAction::Proxy => "Proxy connection failed",
                _ => "Direct relay connection failed",
            };
            fail(title, &error);
            return;
        }
    };

    telemetry.activate(id);
    let proxy = mapping.decision.proxy.as_ref().filter(|_| used_proxy);
    let progress = TransferProgress::default();
    let mut client = Counted {
        stream: client,
        telemetry,
        id,
        progress: &progress,
        health: None,
        is_upstream: false,
    };
    let mut upstream = Counted {
        stream,
        telemetry,
        id,
        progress: &progress,
        health: proxy.map(|proxy| (proxy, generation)),
        is_upstream: true,
    };
    // A separate observer starts the deadline only once the first request byte
    // reaches the upstream. Idle server-first sockets therefore have no deadline.
    let copied = tokio::select! {
        biased;
        () = wait_for_shutdown(cancel) => {
            telemetry.finish(id, None);
            return;
        }
        copied = aio::copy_bidirectional(&mut client, &mut upstream) => copied,
        () = progress.wait_for_response(), if proxy.is_some() => {
            Ok((0, 0))
        }
    };
    let mut blackhole = None;
    if let Some(proxy) = proxy {
        if !progress.responded.load(Ordering::Relaxed)
            && !progress.client_reset.load(Ordering::Relaxed)
        {
            if progress.wrote.load(Ordering::Relaxed) {
                let failure = if progress.upstream_eof.load(Ordering::Relaxed) {
                    UpstreamError::protocol(
                        "Proxy tunnel closed without returning any response bytes",
                    )
                } else if let Err(error) = &copied {
                    classify_relay_io(error)
                } else {
                    UpstreamError::timeout(
                        "Proxy tunnel returned no response bytes before the deadline",
                    )
                };
                telemetry.record_proxy_result(proxy, generation, Err(&failure));
                blackhole = Some(failure);
            } else if copied.is_ok() {
                telemetry.record_proxy_result(proxy, generation, Ok(()));
            }
        }
    }
    match (blackhole, copied) {
        (Some(error), _) => fail("Proxy tunnel failed", &error),
        (_, Err(error)) if !is_graceful_teardown(&error) => {
            fail("Relay I/O failed", &error.into());
        }
        _ => telemetry.finish(id, None),
    }
}

fn classify_relay_io(error: &io::Error) -> UpstreamError {
    UpstreamError::from(io::Error::new(error.kind(), error.to_string()))
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
    generation: u64,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(UpstreamStream, bool), UpstreamError> {
    tokio::select! {
        biased;
        () = wait_for_shutdown(cancel) => Err(io::Error::new(io::ErrorKind::Interrupted, "Relay shutting down").into()),
        upstream = dial_upstream(mapping, telemetry, action, domain, generation) => upstream,
    }
}

async fn dial_upstream(
    mapping: &Mapping,
    telemetry: &Telemetry,
    action: RuleAction,
    domain: Option<&str>,
    generation: u64,
) -> Result<(UpstreamStream, bool), UpstreamError> {
    let remote = mapping.flow.remote;
    if action == RuleAction::Proxy {
        let proxy =
            mapping.decision.proxy.as_ref().ok_or_else(|| {
                UpstreamError::protocol("No enabled upstream proxy is configured")
            })?;
        match upstream::connect(proxy, remote, domain).await {
            Ok(stream) => return Ok((stream, true)),
            Err(error) => {
                telemetry.record_proxy_result(proxy, generation, Err(&error));
                if mapping.decision.network_lock || !proxy.fallback_direct {
                    return Err(error);
                }
                telemetry.log(
                    LogLevel::Warn,
                    LogCategory::Proxy,
                    "Proxy connect failed; retrying directly",
                    Some(format!("{remote}: {error}")),
                );
                telemetry.resolve_route(
                    mapping.id,
                    RuleAction::Direct,
                    mapping.decision.rule_id.is_some(),
                );
                telemetry.route_detail(
                    mapping.id,
                    Some("Fallback direct after proxy failure"),
                    Some(error.code),
                );
            }
        }
    }
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(remote))
        .await
        .map_err(|_| UpstreamError::timeout("Direct connection timed out"))?
        .map_err(UpstreamError::destination)?;
    Ok((UpstreamStream::Tcp(stream), false))
}

#[derive(Default)]
struct TransferProgress {
    wrote: AtomicBool,
    responded: AtomicBool,
    upstream_eof: AtomicBool,
    client_reset: AtomicBool,
    changed: Notify,
}

impl TransferProgress {
    async fn wait_for_response(&self) {
        while !self.wrote.load(Ordering::Relaxed) {
            self.changed.notified().await;
        }
        let _ = tokio::time::timeout(RESPONSE_TIMEOUT, async {
            while !self.upstream_eof.load(Ordering::Relaxed)
                && !self.responded.load(Ordering::Relaxed)
            {
                self.changed.notified().await;
            }
        })
        .await;
        if self.responded.load(Ordering::Relaxed) {
            std::future::pending::<()>().await;
        }
    }

    fn mark(&self, flag: &AtomicBool) -> bool {
        let first = !flag.swap(true, Ordering::Relaxed);
        if first {
            self.changed.notify_waiters();
        }
        first
    }
}

struct Counted<'a, S> {
    stream: S,
    telemetry: &'a Telemetry,
    id: u64,
    progress: &'a TransferProgress,
    health: Option<(&'a ProxyConfig, u64)>,
    is_upstream: bool,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buffer);
        match &result {
            Poll::Ready(Ok(())) if self.is_upstream => {
                if buffer.filled().len() > before {
                    if self.progress.mark(&self.progress.responded) {
                        if let Some((proxy, generation)) = self.health {
                            self.telemetry
                                .record_proxy_result(proxy, generation, Ok(()));
                        }
                    }
                } else {
                    self.progress.mark(&self.progress.upstream_eof);
                }
            }
            Poll::Ready(Err(error)) if !self.is_upstream && is_graceful_teardown(error) => {
                self.progress.mark(&self.progress.client_reset);
            }
            _ => {}
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        match &result {
            Poll::Ready(Ok(count)) => {
                let (up, down) = if self.is_upstream {
                    if *count > 0 {
                        self.progress.mark(&self.progress.wrote);
                    }
                    (*count, 0)
                } else {
                    (0, *count)
                };
                self.telemetry.bytes(self.id, up, down);
            }
            Poll::Ready(Err(error)) if !self.is_upstream && is_graceful_teardown(error) => {
                self.progress.mark(&self.progress.client_reset);
            }
            _ => {}
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.stream).poll_shutdown(cx);
        if matches!(&result, Poll::Ready(Err(error)) if !self.is_upstream && is_graceful_teardown(error))
        {
            self.progress.mark(&self.progress.client_reset);
        }
        result
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
                skip_tls_verify: false,
                fallback_direct: false,
                enabled: true,
            }),
            ..Decision::direct()
        };
        let telemetry = Telemetry::new(None);
        let id = telemetry.open(
            flow,
            &crate::process::Process::unknown(1),
            &decision,
            true,
            None,
        );
        let mapping = Mapping {
            flow,
            id,
            decision,
            bypass_hosts: None,
            interface: [0, 0],
        };
        let (_shutdown, mut cancel) = watch::channel(false);

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
                transfer(accepted, &mapping, &telemetry, &mut cancel)
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

    #[tokio::test]
    async fn zero_byte_tunnel_penalizes_health_but_client_reset_does_not() {
        use shared::{AppConfig, IpcEvent, ProxyHealthStatus, ServiceStatus};

        for client_reset in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let client = TcpStream::connect(listener.local_addr().unwrap())
                .await
                .unwrap();
            let (accepted, _) = listener.accept().await.unwrap();
            let proxy = upstream::mock_proxy(
                ProxyProtocol::Socks5,
                proxy_listener.local_addr().unwrap().port(),
            );
            let telemetry = Telemetry::new(None);
            let mut config = AppConfig::default();
            config.proxies.push(proxy.clone());
            telemetry.update_config(&config);
            let decision = Decision {
                action: RuleAction::Proxy,
                proxy: Some(proxy),
                ..Decision::direct()
            };
            let flow = FlowKey {
                local: client.local_addr().unwrap().to_string().parse().unwrap(),
                remote: "203.0.113.1:443".parse().unwrap(),
            };
            let id = telemetry.open(
                flow,
                &crate::process::Process::unknown(1),
                &decision,
                true,
                None,
            );
            let mapping = Mapping {
                flow,
                id,
                decision,
                bypass_hosts: None,
                interface: [0, 0],
            };
            let (_shutdown, mut cancel) = watch::channel(false);
            let (sent, seen) = oneshot::channel();
            let proxy_side = async {
                let (mut stream, _) = proxy_listener.accept().await.unwrap();
                stream.read_exact(&mut [0; 3]).await.unwrap();
                stream.write_all(&[5, 0]).await.unwrap();
                stream.read_exact(&mut [0; 10]).await.unwrap();
                stream
                    .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                let mut request = [0; 4];
                stream.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                sent.send(()).unwrap();
                if client_reset {
                    assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
                } else {
                    stream.shutdown().await.unwrap();
                }
            };
            let client_side = async {
                let mut client = client;
                client.write_all(b"ping").await.unwrap();
                seen.await.unwrap();
                if client_reset {
                    client.set_zero_linger().unwrap();
                    drop(client);
                } else {
                    // Keep the client write side open. Upstream EOF must end the relay.
                    assert_eq!(client.read(&mut [0; 1]).await.unwrap(), 0);
                }
            };
            timeout(Duration::from_secs(3), async {
                tokio::join!(
                    proxy_side,
                    client_side,
                    transfer(accepted, &mapping, &telemetry, &mut cancel)
                )
            })
            .await
            .unwrap();
            let snapshot = telemetry.connections(true, Some(1)).remove(0);
            let mut events = telemetry.events.subscribe();
            telemetry.tick(&ServiceStatus {
                running: true,
                started_at_ms: None,
                uptime_secs: 1,
                service_pid: 0,
                core_version: "test".into(),
                driver_ready: false,
                active_profile_id: "default".into(),
                last_error: None,
                config_error: None,
                invalid_rule_ids: Vec::new(),
            });
            let IpcEvent::StatsTick(stats) = events.try_recv().unwrap() else {
                panic!("expected stats")
            };
            let health = &stats.proxy_health[0];
            if client_reset {
                assert_eq!(snapshot.state, ConnectionState::Closed);
                assert_eq!(snapshot.failure_code, None);
                assert_eq!(health.status, ProxyHealthStatus::Unknown);
            } else {
                assert_eq!(snapshot.state, ConnectionState::Failed);
                assert_eq!(snapshot.failure_code.as_deref(), Some("protocol_error"));
                assert_eq!(health.status, ProxyHealthStatus::Unreachable);
                assert_eq!(health.last_error_code.as_deref(), Some("protocol_error"));
            }
        }
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
            bypass_hosts: None,
            interface: [0, 0],
        };
        assert!(!nat.remove_unclaimed(flow, 1));
        assert!(nat.insert(key, mapping.clone()));
        assert!(!nat.remove_unclaimed(flow, 2));
        assert!(nat.remove_unclaimed(flow, 1));
        assert!(nat.find(key).is_none());

        assert!(nat.insert(key, mapping));
        assert_eq!(nat.claim(key).unwrap().0.id, 1);
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
            bypass_hosts: None,
            interface: [0, 0],
        };
        let replacement = Mapping {
            id: 2,
            ..original.clone()
        };
        assert!(nat.insert(key, original));
        assert!(!nat.insert(key, replacement.clone()));
        assert_eq!(nat.claim(key).unwrap().0.id, 1);
        assert!(!nat.insert(key, replacement.clone()));

        nat.close(key, 1);
        assert!(nat.insert(key, replacement));
        assert_eq!(nat.claim(key).unwrap().0.id, 2);
        nat.close(key, 1);
        assert!(nat.0.lock().unwrap().get(&key).unwrap().expires.is_none());
    }

    #[test]
    fn clearing_nat_cancels_claimed_relays_and_removes_lingering_entries() {
        let nat = NatTable::default();
        let flow = FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.2:443".parse().unwrap(),
        };
        let lingering_flow = FlowKey {
            local: "192.0.2.1:55001".parse().unwrap(),
            ..flow
        };
        for (flow, id) in [(flow, 1), (lingering_flow, 2)] {
            assert!(nat.insert(
                NatKey::from_flow(flow),
                Mapping {
                    flow,
                    id,
                    decision: Decision::direct(),
                    bypass_hosts: None,
                    interface: [0, 0],
                }
            ));
        }
        let active = NatKey::from_flow(flow);
        let lingering = NatKey::from_flow(lingering_flow);
        let (_, mut retired) = nat.claim(active).unwrap();
        let _ = nat.claim(lingering).unwrap();
        nat.close(lingering, 2);
        assert_eq!(retired.try_recv(), Err(oneshot::error::TryRecvError::Empty));

        let mut ids = nat.clear();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
        assert_eq!(
            retired.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
        assert!(nat.find(active).is_none());
        assert!(nat.find(lingering).is_none());
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
                    skip_tls_verify: false,
                    fallback_direct,
                    enabled: true,
                }),
                ..Decision::direct()
            };
            let telemetry = Telemetry::new(None);
            let id = telemetry.open(
                flow,
                &crate::process::Process::unknown(1),
                &decision,
                true,
                None,
            );
            let mapping = Mapping {
                flow,
                id,
                decision,
                bypass_hosts: None,
                interface: [0, 0],
            };
            let (_shutdown, mut cancel) = watch::channel(false);
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
            let connect = connect_upstream(
                &mapping,
                &telemetry,
                RuleAction::Proxy,
                None,
                0,
                &mut cancel,
            );
            let ((), result) = timeout(Duration::from_secs(2), async {
                tokio::join!(reject, connect)
            })
            .await
            .unwrap();
            let snapshot = telemetry.connections(false, Some(1)).remove(0);
            if fallback_direct && !network_lock {
                let (UpstreamStream::Tcp(stream), false) = result.unwrap() else {
                    panic!("expected a direct TCP fallback")
                };
                assert_eq!(
                    stream.peer_addr().unwrap(),
                    destination.local_addr().unwrap()
                );
                assert_eq!(snapshot.action, RuleAction::Direct);
                assert!(snapshot.proxy_id.is_none());
                assert!(telemetry
                    .logs(None)
                    .iter()
                    .any(|entry| matches!(entry.level, LogLevel::Warn)));
            } else {
                assert!(result.err().unwrap().to_string().contains("reply 0x05"));
                assert_eq!(snapshot.action, RuleAction::Proxy);
                assert!(telemetry.logs(None).is_empty());
            }
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_a_stalled_socks5_handshake() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mapping = Mapping {
            flow: FlowKey {
                local: "192.0.2.1:55000".parse().unwrap(),
                remote: "203.0.113.2:443".parse().unwrap(),
            },
            id: 1,
            decision: Decision {
                action: RuleAction::Proxy,
                proxy: Some(ProxyConfig {
                    id: "stalled".into(),
                    name: "Stalled proxy".into(),
                    protocol: ProxyProtocol::Socks5,
                    host: "127.0.0.1".into(),
                    port: proxy.local_addr().unwrap().port(),
                    auth: None,
                    interface_name: None,
                    allow_lan: false,
                    udp_relay: false,
                    skip_tls_verify: false,
                    fallback_direct: false,
                    enabled: true,
                }),
                ..Decision::direct()
            },
            bypass_hosts: None,
            interface: [0, 0],
        };
        let telemetry = Telemetry::new(None);
        let (shutdown, mut cancel) = watch::channel(false);
        let proxy_side = async {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            let cancelled_at = Instant::now();
            shutdown.send(true).unwrap();
            assert_eq!(
                timeout(Duration::from_millis(300), socket.read(&mut [0; 1]))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
            cancelled_at
        };
        let connect = async {
            let result = connect_upstream(
                &mapping,
                &telemetry,
                RuleAction::Proxy,
                None,
                0,
                &mut cancel,
            )
            .await;
            assert_eq!(result.err().unwrap().io.kind(), io::ErrorKind::Interrupted);
            Instant::now()
        };
        let (cancelled_at, finished_at) = timeout(Duration::from_secs(2), async {
            tokio::join!(proxy_side, connect)
        })
        .await
        .unwrap();
        assert!(finished_at.duration_since(cancelled_at) < Duration::from_millis(300));
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
    async fn relay_dials_original_ip_for_unmatched_excluded_and_bypassed_domains() {
        for (host, action, bypass) in [
            ("other.test", RuleAction::Proxy, false),
            ("ads.example.org", RuleAction::Proxy, false),
            ("ads.example.org", RuleAction::Block, false),
            ("www.example.org", RuleAction::Proxy, true),
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
            let telemetry = Telemetry::new(None);
            let id = telemetry.open(
                flow,
                &crate::process::Process::unknown(1),
                &decision,
                true,
                None,
            );
            let mapping = Mapping {
                flow,
                id,
                decision,
                bypass_hosts: if bypass {
                    BypassHosts::parse(&["*.example.org".into()]).unwrap()
                } else {
                    None
                },
                interface: [0, 0],
            };
            let (_shutdown, mut cancel) = watch::channel(false);
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
                tokio::join!(
                    server,
                    client,
                    transfer(accepted, &mapping, &telemetry, &mut cancel)
                )
            })
            .await
            .unwrap();
            let snapshot = telemetry.connections(true, Some(1)).remove(0);
            assert_eq!(snapshot.domain.as_deref(), Some(host));
            assert_eq!(snapshot.action, RuleAction::Direct);
            assert_eq!(snapshot.state, ConnectionState::Closed);
            assert_eq!(snapshot.rule_id.as_deref(), bypass.then_some("domains"));
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
                skip_tls_verify: false,
                fallback_direct: true,
                enabled: true,
            }),
            ..Decision::direct()
        };
        let telemetry = Telemetry::new(None);
        let id = telemetry.open(
            flow,
            &crate::process::Process::unknown(1),
            &decision,
            true,
            None,
        );
        let nat = Arc::new(NatTable::default());
        assert!(nat.insert(
            NatKey::from_flow(flow),
            Mapping {
                flow,
                id,
                decision,
                bypass_hosts: None,
                interface: [0, 0]
            }
        ));
        let (shutdown, cancel) = watch::channel(false);
        let relay_task = tokio::spawn(run(relay, Arc::clone(&nat), Arc::clone(&telemetry), cancel));
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
        shutdown.send(true).unwrap();
        relay_task.await.unwrap().unwrap();
    }
}
