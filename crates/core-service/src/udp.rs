use std::{
    collections::{HashMap, VecDeque},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
    sync::{Arc, Weak},
    time::Duration,
};

use shared::{ProxyConfig, RuleAction};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::{mpsc, watch},
    task::{AbortHandle, JoinHandle, JoinSet},
    time::{self, Instant},
};

use crate::{
    config::is_private_or_lan_v4,
    packet::{build_inbound_udp, FlowKey, UdpPacket, MAX_UDP_PAYLOAD},
    socks5,
    telemetry::Telemetry,
    upstream::{self, UpstreamError, CONNECT_TIMEOUT},
    windivert::{Address, Divert},
};

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SESSION_LIMIT: usize = 64;
const DNS_TASK_LIMIT: usize = 64;
const DISPATCH_QUEUE: usize = 128;
const SESSION_QUEUE: usize = 8;
const DATAGRAM_LIMIT: usize = 64;
const SOCKS_HEADER: usize = 10;
const PUBLIC_DNS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 53);

pub(crate) enum Path {
    Associate,
    DnsTcp,
}

pub(crate) struct Request {
    pub id: u64,
    pub key: FlowKey,
    pub owner: u32,
    pub address: Address,
    pub bytes: Vec<u8>,
    pub proxy: ProxyConfig,
    pub allow_fallback: bool,
    pub path: Path,
}

#[derive(Clone)]
pub(crate) struct Relay {
    requests: mpsc::Sender<Job>,
    revision: watch::Sender<u64>,
    telemetry: Arc<Telemetry>,
}

impl Relay {
    pub fn start(divert: Weak<Divert>, telemetry: Arc<Telemetry>) -> (Self, JoinHandle<()>) {
        let (requests, queue) = mpsc::channel(DISPATCH_QUEUE);
        let (revision, changes) = watch::channel(0);
        let task = tokio::spawn(dispatch(queue, changes, divert));
        (
            Self {
                requests,
                revision,
                telemetry,
            },
            task,
        )
    }

    pub fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    pub fn reset(&self) {
        self.revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub fn submit(&self, request: Request, revision: u64) {
        let job = Job {
            generation: self.telemetry.health_generation(),
            telemetry: Arc::clone(&self.telemetry),
            request,
            revision,
        };
        if let Err(error) = self.requests.try_send(job) {
            error.into_inner().fail(
                &UpstreamError::protocol("UDP relay dispatch queue is unavailable"),
                false,
            );
        }
    }
}

struct Job {
    request: Request,
    telemetry: Arc<Telemetry>,
    generation: u64,
    revision: u64,
}

impl Job {
    fn payload(&self) -> &[u8] {
        UdpPacket::parse(&self.request.bytes)
            .expect("validated UDP packet")
            .payload
    }

    fn fail(&self, error: &UpstreamError, update_health: bool) {
        if update_health {
            self.telemetry
                .record_proxy_result(&self.request.proxy, self.generation, Err(error));
        }
        self.telemetry
            .route_detail(self.request.id, None, Some(error.code));
        self.telemetry
            .finish(self.request.id, Some(error.to_string()));
    }

    fn success(&self, bytes: usize) {
        self.telemetry
            .record_proxy_result(&self.request.proxy, self.generation, Ok(()));
        self.telemetry.bytes(self.request.id, 0, bytes);
        self.telemetry.finish(self.request.id, None);
    }

    fn upload(&self, bytes: usize) {
        self.telemetry.activate(self.request.id);
        self.telemetry.bytes(self.request.id, bytes, 0);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.telemetry.finish(self.request.id, None);
    }
}

#[derive(Clone)]
struct Injector {
    divert: Weak<Divert>,
    revision: watch::Receiver<u64>,
}

impl Injector {
    fn inbound(
        &self,
        revision: u64,
        address: Address,
        key: FlowKey,
        payload: &[u8],
    ) -> io::Result<bool> {
        self.with_handle(revision, |handle| {
            let mut bytes = build_inbound_udp(key.remote, key.local, payload);
            if bytes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid IPv4 UDP reply length",
                ));
            }
            handle.reflect(&mut bytes, address, address.interface())
        })
    }

    fn direct(&self, job: &Job) -> io::Result<bool> {
        self.with_handle(job.revision, |handle| {
            handle.send(&job.request.bytes, &job.request.address)
        })
    }

    fn with_handle(
        &self,
        revision: u64,
        send: impl FnOnce(&Divert) -> io::Result<()>,
    ) -> io::Result<bool> {
        let current = self.revision.borrow();
        let Some(handle) = self.divert.upgrade().filter(|_| revision == *current) else {
            return Ok(false);
        };
        send(&handle).map(|()| true)
    }
}

struct SessionHandle {
    owner: u32,
    requests: mpsc::Sender<Job>,
    task: AbortHandle,
}

async fn dispatch(
    mut queue: mpsc::Receiver<Job>,
    mut changes: watch::Receiver<u64>,
    divert: Weak<Divert>,
) {
    let injector = Injector {
        divert,
        revision: changes.clone(),
    };
    let mut sessions = HashMap::<(SocketAddrV4, String), SessionHandle>::new();
    let mut pipes = JoinSet::new();
    let mut dns = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            change = changes.changed() => {
                if change.is_err() {
                    return;
                }
                sessions.clear();
                pipes.abort_all();
                dns.abort_all();
            }
            Some(result) = pipes.join_next(), if !pipes.is_empty() => {
                if let Some(error) = result.err().filter(tokio::task::JoinError::is_panic) {
                    tracing::error!(%error, "UDP relay session panicked");
                }
            }
            Some(result) = dns.join_next(), if !dns.is_empty() => {
                if let Some(error) = result.err().filter(tokio::task::JoinError::is_panic) {
                    tracing::error!(%error, "DNS-over-TCP task panicked");
                }
            }
            received = queue.recv() => {
                let Some(job) = received else { return };
                if job.revision != *changes.borrow() {
                    continue;
                }
                match job.request.path {
                    Path::DnsTcp if dns.len() < DNS_TASK_LIMIT => {
                        let injector = injector.clone();
                        dns.spawn(async move { forward_dns(job, &injector).await });
                    }
                    Path::DnsTcp => job.fail(&UpstreamError::protocol("DNS-over-TCP task limit reached"), false),
                    Path::Associate => {
                        sessions.retain(|_, session| !session.requests.is_closed());
                        let key = (job.request.key.local, job.request.proxy.id.clone());
                        if sessions.get(&key).is_some_and(|session| session.owner != job.request.owner) {
                            sessions.remove(&key).expect("session key exists").task.abort();
                        }
                        if let Some(session) = sessions.get(&key) {
                            if let Err(error) = session.requests.try_send(job) {
                                error.into_inner().fail(&UpstreamError::protocol("UDP relay session queue is unavailable"), false);
                            }
                            continue;
                        }
                        if sessions.len() >= SESSION_LIMIT {
                            job.fail(&UpstreamError::protocol("UDP relay session limit reached"), false);
                            continue;
                        }
                        let owner = job.request.owner;
                        let (requests, jobs) = mpsc::channel(SESSION_QUEUE);
                        let task = pipes.spawn(run_session(job, jobs, injector.clone()));
                        sessions.insert(key, SessionHandle { owner, requests, task });
                    }
                }
            }
        }
    }
}

async fn run_session(first: Job, mut jobs: mpsc::Receiver<Job>, injector: Injector) {
    let association = match Association::open(&first.request.proxy).await {
        Ok(association) => association,
        Err(error) if error.io.kind() == io::ErrorKind::Unsupported => {
            let mut job = first;
            loop {
                if job.request.key.remote.port() == 53 {
                    forward_dns(job, &injector).await;
                } else if job.request.allow_fallback {
                    forward_direct(job, &injector, error.code);
                } else {
                    job.fail(&error, true);
                }
                let Ok(Some(next)) = time::timeout(IDLE_TIMEOUT, jobs.recv()).await else {
                    return;
                };
                job = next;
            }
        }
        Err(error) => {
            first.fail(&error, true);
            jobs.close();
            while let Some(job) = jobs.recv().await {
                job.fail(&error, false);
            }
            return;
        }
    };
    association.run(first, jobs, &injector).await;
}

fn forward_direct(job: Job, injector: &Injector, code: &str) {
    let id = job.request.id;
    job.telemetry.resolve_route(id, RuleAction::Direct, true);
    job.telemetry
        .route_detail(id, Some("Fallback direct after proxy failure"), Some(code));
    let len = job.payload().len();
    match injector.direct(&job) {
        Ok(true) => job.upload(len),
        Ok(false) => {}
        Err(error) => job.fail(&error.into(), false),
    }
}

async fn forward_dns(job: Job, injector: &Injector) {
    let key = job.request.key;
    job.telemetry
        .route_detail(job.request.id, Some("DNS-over-TCP via proxy"), None);
    let payload = job.payload();
    let resolver = if is_private_or_lan_v4(*key.remote.ip()) {
        PUBLIC_DNS
    } else {
        key.remote
    };
    let reply = dns_over_tcp(&job.request.proxy, resolver, payload, || {
        job.upload(payload.len())
    })
    .await;
    match reply {
        Ok(reply) => match injector.inbound(job.revision, job.request.address, key, &reply) {
            Ok(true) => job.success(reply.len()),
            Ok(false) => {}
            Err(error) => job.fail(&error.into(), false),
        },
        Err(error) => job.fail(&error, true),
    }
}

async fn dns_over_tcp(
    proxy: &ProxyConfig,
    resolver: SocketAddrV4,
    query: &[u8],
    sent: impl FnOnce(),
) -> Result<Vec<u8>, UpstreamError> {
    if query.len() < 12 || query.len() > MAX_UDP_PAYLOAD || query[2] & 0x80 != 0 {
        return Err(UpstreamError {
            code: "protocol_error",
            is_proxy_fault: false,
            io: io::Error::new(io::ErrorKind::InvalidData, "Invalid DNS query"),
        });
    }
    time::timeout(CONNECT_TIMEOUT, async {
        let mut stream = upstream::connect(proxy, resolver, None).await?;
        stream.write_u16(query.len() as u16).await?;
        stream.write_all(query).await?;
        stream.flush().await?;
        sent();
        let len = usize::from(stream.read_u16().await?);
        if !(12..=MAX_UDP_PAYLOAD).contains(&len) {
            return Err(UpstreamError::protocol(
                "Invalid DNS-over-TCP response length",
            ));
        }
        let mut response = vec![0; len];
        stream.read_exact(&mut response).await?;
        if response[..2] != query[..2] || response[2] & 0x80 == 0 {
            return Err(UpstreamError::protocol(
                "DNS-over-TCP response does not match the query",
            ));
        }
        Ok(response)
    })
    .await
    .map_err(|_| UpstreamError::timeout("DNS-over-TCP timed out"))?
}

struct Association {
    control: TcpStream,
    socket: UdpSocket,
}

impl Association {
    async fn open(proxy: &ProxyConfig) -> Result<Self, UpstreamError> {
        let (control, endpoint) = socks5::associate(proxy).await?;
        let local = control.local_addr()?;
        let source = if local.is_ipv4() == endpoint.is_ipv4() {
            let mut source = local;
            source.set_port(0);
            // Some local SOCKS servers use an external UDP listener.
            if source.ip().is_loopback() == endpoint.ip().is_loopback() {
                source
            } else {
                SocketAddr::new(unspecified(endpoint), 0)
            }
        } else {
            SocketAddr::new(unspecified(endpoint), 0)
        };
        let socket = UdpSocket::bind(source).await?;
        socket.connect(endpoint).await?;
        Ok(Self { control, socket })
    }

    async fn run(mut self, first: Job, mut jobs: mpsc::Receiver<Job>, injector: &Injector) {
        let mut destinations = HashMap::<SocketAddrV4, ReplyRoute>::new();
        let mut pending = VecDeque::new();
        self.send(first, &mut destinations, &mut pending).await;
        let mut buffer = vec![0; MAX_UDP_PAYLOAD];
        let mut control_byte = [0];
        let idle = time::sleep(IDLE_TIMEOUT);
        tokio::pin!(idle);
        let mut expire = time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = &mut idle => break,
                result = self.control.read(&mut control_byte) => {
                    let error = result.err().map_or_else(
                        || UpstreamError::protocol("SOCKS5 UDP control connection closed"), UpstreamError::from);
                    if let Some(datagram) = pending.front() {
                        datagram.job.fail(&error, true);
                    }
                    break;
                }
                result = self.socket.recv(&mut buffer) => match result {
                    Ok(len) => {
                        if Self::receive(&buffer[..len], &destinations, &mut pending, injector) {
                            idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                        }
                    }
                    Err(error) => {
                        if let Some(datagram) = pending.front() {
                            datagram.job.fail(&error.into(), true);
                        }
                        break;
                    }
                },
                next = jobs.recv() => {
                    let Some(job) = next else { break };
                    self.send(job, &mut destinations, &mut pending).await;
                    idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                }
                _ = expire.tick() => {
                    pending.retain(|datagram| datagram.sent.elapsed() < IDLE_TIMEOUT);
                    destinations.retain(|_, route| route.last_sent.elapsed() < IDLE_TIMEOUT);
                }
            }
        }
    }

    async fn send(
        &self,
        mut job: Job,
        destinations: &mut HashMap<SocketAddrV4, ReplyRoute>,
        pending: &mut VecDeque<Datagram>,
    ) {
        let remote = job.request.key.remote;
        let payload = job.payload();
        let Some(packet) = encode_datagram(remote, payload) else {
            job.fail(
                &UpstreamError::protocol("UDP payload exceeds SOCKS5 relay limit"),
                false,
            );
            return;
        };
        match time::timeout(CONNECT_TIMEOUT, self.socket.send(&packet)).await {
            Ok(Ok(sent)) if sent == packet.len() => job.upload(payload.len()),
            Ok(Ok(_)) => {
                job.fail(
                    &UpstreamError::protocol("Incomplete SOCKS5 UDP datagram send"),
                    true,
                );
                return;
            }
            Ok(Err(error)) => {
                job.fail(&error.into(), true);
                return;
            }
            Err(_) => {
                job.fail(&UpstreamError::timeout("SOCKS5 UDP send timed out"), true);
                return;
            }
        }
        let dns_id = (remote.port() == 53).then(|| dns_id(payload)).flatten();
        let sent = Instant::now();
        if destinations.len() == DATAGRAM_LIMIT && !destinations.contains_key(&remote) {
            if let Some((&oldest, _)) = destinations.iter().min_by_key(|(_, route)| route.last_sent)
            {
                destinations.remove(&oldest);
            }
        }
        destinations.insert(
            remote,
            ReplyRoute {
                key: job.request.key,
                address: job.request.address,
                revision: job.revision,
                last_sent: sent,
            },
        );
        if pending.len() == DATAGRAM_LIMIT {
            pending.pop_front();
        }
        job.request.bytes = Vec::new();
        pending.push_back(Datagram { job, dns_id, sent });
    }

    fn receive(
        packet: &[u8],
        destinations: &HashMap<SocketAddrV4, ReplyRoute>,
        pending: &mut VecDeque<Datagram>,
        injector: &Injector,
    ) -> bool {
        let Some((remote, payload)) = decode_datagram(packet) else {
            return false;
        };
        let Some(route) = destinations
            .get(&remote)
            .filter(|route| route.last_sent.elapsed() < IDLE_TIMEOUT)
        else {
            return false;
        };
        let dns = remote.port() == 53;
        let reply_id = dns_id(payload);
        let index = pending.iter().position(|datagram| {
            datagram.job.request.key.remote == remote && (!dns || datagram.dns_id == reply_id)
        });
        if dns && (index.is_none() || reply_id.is_none() || payload[2] & 0x80 == 0) {
            return false;
        }
        let result = injector.inbound(route.revision, route.address, route.key, payload);
        if let Some(job) = index
            .and_then(|index| pending.remove(index))
            .map(|datagram| datagram.job)
        {
            match result {
                Ok(true) => job.success(payload.len()),
                Ok(false) => {}
                Err(error) => {
                    job.fail(&error.into(), false);
                    return false;
                }
            }
        } else if let Err(error) = result {
            tracing::debug!(%error, "Could not inject SOCKS5 UDP reply");
            return false;
        }
        true
    }
}

struct Datagram {
    job: Job,
    dns_id: Option<u16>,
    sent: Instant,
}

struct ReplyRoute {
    key: FlowKey,
    address: Address,
    revision: u64,
    last_sent: Instant,
}

fn unspecified(address: SocketAddr) -> IpAddr {
    if address.is_ipv4() {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        Ipv6Addr::UNSPECIFIED.into()
    }
}

fn dns_id(payload: &[u8]) -> Option<u16> {
    (payload.len() >= 12).then(|| u16::from_be_bytes([payload[0], payload[1]]))
}

fn encode_datagram(remote: SocketAddrV4, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_UDP_PAYLOAD - SOCKS_HEADER {
        return None;
    }
    let mut packet = Vec::with_capacity(SOCKS_HEADER + payload.len());
    packet.extend([0, 0, 0, 1]);
    packet.extend(remote.ip().octets());
    packet.extend(remote.port().to_be_bytes());
    packet.extend(payload);
    Some(packet)
}

fn decode_datagram(packet: &[u8]) -> Option<(SocketAddrV4, &[u8])> {
    // The SOCKS5 UDP FRAG extension requires reassembly and is optional.
    if packet.get(..3)? != [0, 0, 0] {
        return None;
    }
    let (ip, offset) = match packet.get(3)? {
        1 => (
            Ipv4Addr::from(<[u8; 4]>::try_from(packet.get(4..8)?).ok()?),
            8,
        ),
        4 => (
            Ipv6Addr::from(<[u8; 16]>::try_from(packet.get(4..20)?).ok()?).to_ipv4_mapped()?,
            20,
        ),
        _ => return None,
    };
    let port = u16::from_be_bytes(packet.get(offset..offset + 2)?.try_into().ok()?);
    Some((SocketAddrV4::new(ip, port), &packet[offset + 2..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ProxyProtocol;
    use tokio::net::TcpListener;

    use crate::{packet::parse_dns_query_name, upstream::mock_proxy};

    fn dns_query() -> Vec<u8> {
        b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x03www\x07example\x03org\x00\x00\x01\x00\x01".to_vec()
    }

    async fn socks_greeting(stream: &mut TcpStream) {
        let mut greeting = [0; 3];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 0]);
        stream.write_all(&[5, 0]).await.unwrap();
    }

    #[tokio::test]
    async fn socks5_udp_associate_keeps_control_stream_and_roundtrips_datagrams() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy = mock_proxy(ProxyProtocol::Socks5, listener.local_addr().unwrap().port());
        let remote: SocketAddrV4 = "203.0.113.7:123".parse().unwrap();
        let local: SocketAddrV4 = "192.0.2.3:50000".parse().unwrap();
        let server = async {
            let (mut control, _) = listener.accept().await.unwrap();
            socks_greeting(&mut control).await;
            let mut command = [0; 10];
            control.read_exact(&mut command).await.unwrap();
            assert_eq!(command, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
            let mut response = vec![5, 0, 0, 1, 0, 0, 0, 0];
            response.extend(relay.local_addr().unwrap().port().to_be_bytes());
            control.write_all(&response).await.unwrap();
            for payload in [&b"hello"[..], &b"again"[..]] {
                let mut datagram = [0; 40];
                let (len, from) = relay.recv_from(&mut datagram).await.unwrap();
                assert_eq!(decode_datagram(&datagram[..len]), Some((remote, payload)));
                relay
                    .send_to(&encode_datagram(remote, b"reply").unwrap(), from)
                    .await
                    .unwrap();
            }
            assert_eq!(control.read(&mut [0]).await.unwrap(), 0);
        };
        let client = async {
            let association = Association::open(&proxy).await.unwrap();
            for payload in [&b"hello"[..], &b"again"[..]] {
                association
                    .socket
                    .send(&encode_datagram(remote, payload).unwrap())
                    .await
                    .unwrap();
                let mut datagram = [0; 40];
                let len = association.socket.recv(&mut datagram).await.unwrap();
                let (source, reply) = decode_datagram(&datagram[..len]).unwrap();
                let inbound = build_inbound_udp(source, local, reply);
                let parsed = UdpPacket::parse(&inbound).unwrap();
                assert_eq!(
                    parsed.key,
                    FlowKey {
                        local: remote,
                        remote: local
                    }
                );
                assert_eq!(parsed.payload, b"reply");
            }
        };
        time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client)
        })
        .await
        .unwrap();
        let mut fragmented = encode_datagram(remote, b"test").unwrap();
        fragmented[2] = 1;
        assert!(decode_datagram(&fragmented).is_none());
        assert!(encode_datagram(remote, &vec![0; MAX_UDP_PAYLOAD]).is_none());
    }

    #[tokio::test]
    async fn dns_fallback_with_udp_disabled_tunnels_a_length_prefixed_tcp_query() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = mock_proxy(ProxyProtocol::Socks5, listener.local_addr().unwrap().port());
        assert!(!proxy.udp_relay);
        let resolver: SocketAddrV4 = "9.9.9.9:53".parse().unwrap();
        let query = dns_query();
        assert_eq!(
            parse_dns_query_name(&query).as_deref(),
            Some("www.example.org")
        );
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            socks_greeting(&mut stream).await;
            let mut command = [0; 10];
            stream.read_exact(&mut command).await.unwrap();
            assert_eq!(command, [5, 1, 0, 1, 9, 9, 9, 9, 0, 53]);
            stream
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            let sz = stream.read_u16().await.unwrap();
            let mut response = vec![0; usize::from(sz)];
            stream.read_exact(&mut response).await.unwrap();
            assert_eq!(response, query);
            response[2] |= 0x80;
            stream.write_u16(sz).await.unwrap();
            stream.write_all(&response).await.unwrap();
        };
        let sent = std::cell::Cell::new(false);
        let (_, response) = time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                server,
                dns_over_tcp(&proxy, resolver, &query, || sent.set(true))
            )
        })
        .await
        .unwrap();
        assert!(sent.get());
        let response = response.unwrap();
        assert_eq!(&response[..2], &query[..2]);
        assert_eq!(response[2], query[2] | 0x80);
        let original: SocketAddrV4 = "192.168.1.1:53".parse().unwrap();
        let app: SocketAddrV4 = "192.168.1.12:63000".parse().unwrap();
        let inbound = build_inbound_udp(original, app, &response);
        let parsed = UdpPacket::parse(&inbound).unwrap();
        assert_eq!(
            parsed.key,
            FlowKey {
                local: original,
                remote: app
            }
        );
        assert_eq!(parsed.payload, response);
    }

    #[tokio::test]
    async fn rejected_udp_association_is_identified_as_unavailable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = mock_proxy(ProxyProtocol::Socks5, listener.local_addr().unwrap().port());
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            socks_greeting(&mut stream).await;
            stream.read_exact(&mut [0; 10]).await.unwrap();
            stream
                .write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
        };
        let ((), result) = time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, Association::open(&proxy))
        })
        .await
        .unwrap();
        let error = result.err().expect("UDP is unsupported");
        assert_eq!(error.io.kind(), io::ErrorKind::Unsupported);
        assert_eq!(error.code, "protocol_error");
    }
}
