use std::{
    fmt, io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring,
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use shared::{ProxyConfig, ProxyProtocol, ProxyTestResult};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf},
    net::{lookup_host, TcpSocket, TcpStream},
    time::timeout,
};
use tokio_rustls::{client::TlsStream, TlsConnector};

use crate::{network_interfaces, sniffer, socks5};

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HEADER_LIMIT: usize = 16 * 1024;
const PROBE_TARGET: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 80);

pub(crate) enum UpstreamStream {
    Tcp(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    // The reader may have already received tunneled bytes in the CONNECT response.
    Buffered(Box<BufReader<UpstreamStream>>),
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

impl UpstreamStream {
    fn inner(&mut self) -> Pin<&mut dyn Stream> {
        match self {
            Self::Tcp(stream) => Pin::new(stream),
            Self::Tls(stream) => Pin::new(stream.as_mut()),
            Self::Buffered(stream) => Pin::new(stream.as_mut()),
        }
    }
}

impl AsyncRead for UpstreamStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.inner().poll_read(cx, buffer)
    }
}

impl AsyncWrite for UpstreamStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.inner().poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner().poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner().poll_shutdown(cx)
    }
}

#[derive(Debug)]
pub(crate) struct UpstreamError {
    pub code: &'static str,
    pub is_proxy_fault: bool,
    pub io: io::Error,
}

impl UpstreamError {
    pub fn proxy(code: &'static str, io: io::Error) -> Self {
        Self {
            code,
            is_proxy_fault: true,
            io,
        }
    }

    pub fn destination(io: io::Error) -> Self {
        Self {
            code: "dest_refused",
            is_proxy_fault: false,
            io,
        }
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::proxy(
            "protocol_error",
            io::Error::new(io::ErrorKind::InvalidData, message.into()),
        )
    }

    pub fn timeout(message: &str) -> Self {
        Self::proxy("timeout", io::Error::new(io::ErrorKind::TimedOut, message))
    }
}

impl From<io::Error> for UpstreamError {
    fn from(error: io::Error) -> Self {
        let code = if error.kind() == io::ErrorKind::TimedOut {
            "timeout"
        } else {
            "protocol_error"
        };
        Self::proxy(code, error)
    }
}

impl fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.io.fmt(f)
    }
}

impl std::error::Error for UpstreamError {}

fn host(proxy: &ProxyConfig) -> &str {
    proxy
        .host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&proxy.host)
}

pub(crate) async fn connect(
    proxy: &ProxyConfig,
    destination: SocketAddrV4,
    domain: Option<&str>,
) -> Result<UpstreamStream, UpstreamError> {
    timeout(
        CONNECT_TIMEOUT,
        connect_inner(proxy, destination, domain, None),
    )
    .await
    .map_err(|_| UpstreamError::timeout("Upstream connection or handshake timed out"))?
}

async fn connect_inner(
    proxy: &ProxyConfig,
    destination: SocketAddrV4,
    domain: Option<&str>,
    tls_override: Option<Arc<ClientConfig>>,
) -> Result<UpstreamStream, UpstreamError> {
    match proxy.protocol {
        ProxyProtocol::Socks5 => Ok(UpstreamStream::Tcp(
            socks5::connect(proxy, destination, domain).await?,
        )),
        ProxyProtocol::DirectInterface => {
            let interface = proxy.interface_name.as_deref().ok_or_else(|| {
                UpstreamError::protocol("DirectInterface requires a network interface")
            })?;
            let source = interface_address(interface).await?;
            let stream =
                connect_bound(source, destination)
                    .await
                    .map_err(|error| match error.kind() {
                        io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::HostUnreachable
                        | io::ErrorKind::NetworkUnreachable => UpstreamError::destination(error),
                        _ => error.into(),
                    })?;
            Ok(UpstreamStream::Tcp(stream))
        }
        ProxyProtocol::Http | ProxyProtocol::Https => {
            let tcp = connect_proxy(proxy).await?;
            tcp.set_nodelay(true)?;
            let stream = if proxy.protocol == ProxyProtocol::Https {
                let cert_error = |error| UpstreamError::proxy("tls_cert_error", error);
                let name = ServerName::try_from(host(proxy).to_owned()).map_err(|error| {
                    cert_error(io::Error::new(io::ErrorKind::InvalidInput, error))
                })?;
                let config = match tls_override {
                    Some(config) => config,
                    None => tls_config(proxy.skip_tls_verify).await?,
                };
                let stream = TlsConnector::from(config)
                    .connect(name, tcp)
                    .await
                    .map_err(cert_error)?;
                UpstreamStream::Tls(Box::new(stream))
            } else {
                UpstreamStream::Tcp(tcp)
            };
            http_connect(stream, proxy, destination, domain).await
        }
    }
}

pub(crate) async fn connect_proxy(proxy: &ProxyConfig) -> io::Result<TcpStream> {
    let host = host(proxy);
    let Some(interface) = proxy.interface_name.as_deref() else {
        return TcpStream::connect((host, proxy.port)).await;
    };

    let mut source = None;
    let mut last_error = io::Error::new(
        io::ErrorKind::AddrNotAvailable,
        "Proxy host resolved to no addresses",
    );
    let mut addresses: Vec<_> = lookup_host((host, proxy.port))
        .await?
        .map(|remote| match remote {
            SocketAddr::V6(v6) => v6
                .ip()
                .to_ipv4_mapped()
                .map_or(remote, |ip| SocketAddrV4::new(ip, v6.port()).into()),
            _ => remote,
        })
        .collect();
    addresses.sort_by_key(SocketAddr::is_ipv6);
    for remote in addresses {
        let result = if remote.ip().is_loopback() {
            // An adapter-bound IPv4 socket cannot reach a loopback proxy.
            TcpStream::connect(remote).await
        } else if let SocketAddr::V4(remote) = remote {
            if source.is_none() {
                source = Some(interface_address(interface).await);
            }
            match source.as_ref().expect("interface lookup completed") {
                Ok(ip) => connect_bound(*ip, remote).await,
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        } else {
            Err(io::Error::new(io::ErrorKind::AddrNotAvailable, format!(
                "Proxy address {remote} cannot be reached using the selected IPv4 interface {interface:?}"
            )))
        };
        match result {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

async fn interface_address(name: &str) -> io::Result<Ipv4Addr> {
    let interfaces = tokio::task::spawn_blocking(network_interfaces::list)
        .await
        .map_err(io::Error::other)??;
    let interface = interfaces.into_iter().find(|interface| interface.name == name && interface.is_up)
        .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable,
            format!("Selected proxy interface {name:?} is unavailable or has no active IPv4 address")))?;
    interface
        .ipv4
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn connect_bound(source: Ipv4Addr, remote: SocketAddrV4) -> io::Result<TcpStream> {
    let socket = TcpSocket::new_v4()?;
    socket.bind(SocketAddrV4::new(source, 0).into())?;
    socket.connect(remote.into()).await
}

async fn http_connect(
    mut stream: UpstreamStream,
    proxy: &ProxyConfig,
    destination: SocketAddrV4,
    domain: Option<&str>,
) -> Result<UpstreamStream, UpstreamError> {
    let authority = domain.and_then(sniffer::hostname).map_or_else(
        || destination.to_string(),
        |name| format!("{name}:{}", destination.port()),
    );
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(auth) = &proxy.auth {
        let encoded = STANDARD.encode(format!("{}:{}", auth.username, auth.password));
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let status = read_http_head(&mut reader).await?;
    match status {
        200 if reader.buffer().is_empty() => Ok(reader.into_inner()),
        200 => Ok(UpstreamStream::Buffered(Box::new(reader))),
        407 => Err(UpstreamError::proxy(
            "auth_failed",
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "HTTP proxy authentication failed (407)",
            ),
        )),
        502 => Err(UpstreamError::destination(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "HTTP proxy could not reach the destination (502)",
        ))),
        504 => Err(UpstreamError::timeout("HTTP proxy CONNECT timed out (504)")),
        // 503 may be generated locally by an overloaded proxy rather than the destination.
        _ => Err(UpstreamError::protocol(format!(
            "HTTP CONNECT failed ({status})"
        ))),
    }
}

async fn read_http_head(reader: &mut BufReader<UpstreamStream>) -> Result<u16, UpstreamError> {
    let mut head = Vec::with_capacity(512);
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            return Err(UpstreamError::protocol(
                "EOF before the HTTP response headers",
            ));
        }
        let mut used = 0;
        for &byte in &bytes[..bytes.len().min(HEADER_LIMIT - head.len())] {
            head.push(byte);
            used += 1;
            if head.ends_with(b"\r\n\r\n") {
                reader.consume(used);
                let end = head.iter().position(|&byte| byte == b'\r').unwrap_or(0);
                let status = std::str::from_utf8(&head[..end]).ok().and_then(|line| {
                    let mut fields = line.split(' ');
                    if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
                        return None;
                    }
                    let code = fields.next()?;
                    (code.len() == 3)
                        .then(|| code.parse::<u16>().ok())
                        .flatten()
                });
                return status
                    .filter(|code| (100..=599).contains(code))
                    .ok_or_else(|| {
                        UpstreamError::protocol("Invalid HTTP proxy response status line")
                    });
            }
        }
        reader.consume(used);
        if head.len() == HEADER_LIMIT {
            return Err(UpstreamError::protocol(
                "HTTP response headers exceeded the size limit",
            ));
        }
    }
}

async fn tls_config(skip_verify: bool) -> Result<Arc<ClientConfig>, UpstreamError> {
    static VERIFIED: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    static UNVERIFIED: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if skip_verify {
        return Ok(UNVERIFIED
            .get_or_init(|| {
                let provider = Arc::new(ring::default_provider());
                Arc::new(
                    ClientConfig::builder_with_provider(provider.clone())
                        .with_safe_default_protocol_versions()
                        .expect("ring supports default TLS versions")
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(SkipCertificateVerification(
                            provider,
                        )))
                        .with_no_client_auth(),
                )
            })
            .clone());
    }
    if let Some(config) = VERIFIED.get() {
        return Ok(config.clone());
    }
    let config = tokio::task::spawn_blocking(|| VERIFIED.get_or_init(|| {
        let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        if !native.errors.is_empty() {
            tracing::debug!(errors = ?native.errors, "Could not load some system root certificates");
        }
        roots.add_parsable_certificates(native.certs);
        Arc::new(verified_config(roots))
    }).clone()).await.map_err(io::Error::other)?;
    Ok(config)
}

fn verified_config(roots: RootCertStore) -> ClientConfig {
    ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[derive(Debug)]
struct SkipCertificateVerification(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for SkipCertificateVerification {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

pub(crate) async fn test_proxy(proxy: &ProxyConfig) -> ProxyTestResult {
    test_proxy_with_target(proxy, PROBE_TARGET).await
}

pub(crate) async fn test_proxy_with_target(
    proxy: &ProxyConfig,
    target: SocketAddrV4,
) -> ProxyTestResult {
    let start = Instant::now();
    let result = timeout(CONNECT_TIMEOUT, async {
        let mut stream = connect(proxy, target, None).await?;
        stream
            .write_all(
                format!("HEAD / HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        stream.flush().await?;
        read_http_head(&mut BufReader::new(stream)).await
    })
    .await
    .unwrap_or_else(|_| Err(UpstreamError::timeout("End-to-end proxy probe timed out")));
    ProxyTestResult {
        proxy_id: proxy.id.clone(),
        reachable: result.is_ok(),
        latency_ms: result.is_ok().then(|| start.elapsed().as_millis() as u32),
        error: result.err().map(|error| error.to_string()),
        checked_at_ms: crate::now_ms(),
        stale: false,
    }
}

#[cfg(test)]
pub(crate) fn mock_proxy(protocol: ProxyProtocol, port: u16) -> ProxyConfig {
    ProxyConfig {
        id: "mock".into(),
        name: "Mock".into(),
        protocol,
        host: "127.0.0.1".into(),
        port,
        auth: None,
        interface_name: None,
        allow_lan: false,
        udp_relay: false,
        fallback_direct: false,
        skip_tls_verify: false,
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{pki_types::PrivatePkcs8KeyDer, ServerConfig};
    use shared::{AppConfig, ProxyAuth};
    use tokio::{io::AsyncReadExt, net::TcpListener, time::sleep};
    use tokio_rustls::TlsAcceptor;

    const TEST_ROOT: &[u8] = include_bytes!("../tests/fixtures/proxy-test-root.der");
    const TEST_LEAF: &[u8] = include_bytes!("../tests/fixtures/proxy-test-leaf.der");
    const TEST_LEAF_KEY: &[u8] = include_bytes!("../tests/fixtures/proxy-test-leaf-key.der");
    const TEST_SELFSIGNED: &[u8] = include_bytes!("../tests/fixtures/proxy-test-selfsigned.der");
    const TEST_SELFSIGNED_KEY: &[u8] =
        include_bytes!("../tests/fixtures/proxy-test-selfsigned-key.der");
    const DESTINATION: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 443);

    async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> String {
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") && request.len() < 2048 {
            request.push(stream.read_u8().await.unwrap());
        }
        String::from_utf8(request).unwrap()
    }

    #[tokio::test]
    async fn http_connect_prefers_domain_sends_basic_auth_and_preserves_read_ahead() {
        for use_auth in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut proxy = mock_proxy(ProxyProtocol::Http, listener.local_addr().unwrap().port());
            proxy.auth = use_auth.then(|| ProxyAuth {
                username: "alice".into(),
                password: "test".into(),
            });
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                assert!(request.starts_with(
                    "CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n"
                ));
                assert_eq!(
                    request.contains("Proxy-Authorization: Basic YWxpY2U6dGVzdA==\r\n"),
                    use_auth
                );
                stream.write_all(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: mock\r\n\r\ninitial tunnel bytes").await.unwrap();
                let mut payload = [0; 4];
                stream.read_exact(&mut payload).await.unwrap();
                assert_eq!(&payload, b"ping");
            };
            let client = async {
                let mut stream = connect(&proxy, DESTINATION, Some("example.test"))
                    .await
                    .unwrap();
                let mut buf = [0; 20];
                stream.read_exact(&mut buf).await.unwrap();
                assert_eq!(&buf, b"initial tunnel bytes");
                stream.write_all(b"ping").await.unwrap();
            };
            timeout(Duration::from_secs(3), async {
                tokio::join!(server, client)
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn http_connect_classifies_upstream_and_destination_failures() {
        for (status, code, is_proxy_fault) in [
            ("407", "auth_failed", true),
            ("502", "dest_refused", false),
            ("503", "protocol_error", true),
            ("504", "timeout", true),
            ("invalid", "protocol_error", true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy = mock_proxy(ProxyProtocol::Http, listener.local_addr().unwrap().port());
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                assert!(read_request(&mut stream)
                    .await
                    .starts_with("CONNECT 203.0.113.7:443 "));
                stream
                    .write_all(format!("HTTP/1.1 {status} Test\r\n\r\n").as_bytes())
                    .await
                    .unwrap();
            };
            let ((), result) = timeout(Duration::from_secs(3), async {
                tokio::join!(server, connect(&proxy, DESTINATION, None))
            })
            .await
            .unwrap();
            let error = result.err().expect("CONNECT should fail");
            assert_eq!((error.code, error.is_proxy_fault), (code, is_proxy_fault));
        }
    }

    #[tokio::test]
    async fn https_connect_verifies_trusted_cert_and_honors_explicit_opt_out() {
        for (trusted, skip_verify, host) in [
            (true, false, "localhost"),
            (true, false, "127.0.0.1"),
            (false, false, "127.0.0.1"),
            (false, true, "127.0.0.1"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut proxy = mock_proxy(ProxyProtocol::Https, listener.local_addr().unwrap().port());
            proxy.host = host.into();
            proxy.skip_tls_verify = skip_verify;
            let (cert, key) = if trusted {
                (TEST_LEAF, TEST_LEAF_KEY)
            } else {
                (TEST_SELFSIGNED, TEST_SELFSIGNED_KEY)
            };
            let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(cert)],
                    PrivatePkcs8KeyDer::from(key).into(),
                )
                .unwrap();
            let expect_success = trusted || skip_verify;
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let result = TlsAcceptor::from(Arc::new(config)).accept(stream).await;
                if !expect_success {
                    assert!(result.is_err());
                    return;
                }
                let mut stream = result.unwrap();
                assert!(read_request(&mut stream)
                    .await
                    .starts_with("CONNECT 203.0.113.7:443 "));
                stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nready")
                    .await
                    .unwrap();
                stream.flush().await.unwrap();
            };
            let client = async {
                let result = if trusted {
                    let mut roots = RootCertStore::empty();
                    roots.add(CertificateDer::from(TEST_ROOT)).unwrap();
                    connect_inner(
                        &proxy,
                        DESTINATION,
                        None,
                        Some(Arc::new(verified_config(roots))),
                    )
                    .await
                } else {
                    connect(&proxy, DESTINATION, None).await
                };
                if expect_success {
                    let mut stream = result.unwrap();
                    let mut bytes = [0; 5];
                    stream.read_exact(&mut bytes).await.unwrap();
                    assert_eq!(&bytes, b"ready");
                } else {
                    let error = result
                        .err()
                        .expect("self-signed cert should fail verification");
                    assert_eq!((error.code, error.is_proxy_fault), ("tls_cert_error", true));
                }
            };
            timeout(Duration::from_secs(5), async {
                tokio::join!(server, client)
            })
            .await
            .unwrap();
        }
    }

    #[test]
    fn direct_interface_validation_requires_an_adapter_instead_of_an_endpoint() {
        let mut config = AppConfig::default();
        config
            .proxies
            .push(mock_proxy(ProxyProtocol::DirectInterface, 0));
        config.proxies[0].host.clear();
        for name in [None, Some("  ".into())] {
            config.proxies[0].interface_name = name;
            assert!(crate::config::validate(&config).is_err());
        }
        config.proxies[0].interface_name = Some("Ethernet".into());
        assert!(crate::config::validate(&config).is_ok());
    }

    #[tokio::test]
    async fn loopback_proxy_endpoint_bypasses_selected_interface_binding() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut proxy = mock_proxy(ProxyProtocol::Socks5, listener.local_addr().unwrap().port());
        proxy.interface_name = Some("missing adapter".into());
        for host in ["127.0.0.1", "localhost", "::ffff:127.0.0.1"] {
            proxy.host = host.into();
            let stream = timeout(Duration::from_secs(2), connect_proxy(&proxy))
                .await
                .unwrap()
                .unwrap();
            let (_, peer) = listener.accept().await.unwrap();
            assert!(stream.peer_addr().unwrap().ip().is_loopback());
            assert!(peer.ip().is_loopback());
        }
    }

    #[tokio::test]
    async fn proxy_probe_waits_for_the_target_response_through_socks5() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let SocketAddr::V4(probe_target) = target.local_addr().unwrap() else {
            unreachable!()
        };
        let proxy = mock_proxy(ProxyProtocol::Socks5, listener.local_addr().unwrap().port());
        let server = async {
            let (mut client, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            client.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            client.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 10];
            client.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..8], &[5, 1, 0, 1, 127, 0, 0, 1]);
            assert_eq!(
                u16::from_be_bytes([request[8], request[9]]),
                probe_target.port()
            );
            let mut remote = TcpStream::connect(probe_target).await.unwrap();
            client
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
        };
        let origin = async {
            let (mut stream, _) = target.accept().await.unwrap();
            assert!(read_request(&mut stream)
                .await
                .starts_with("HEAD / HTTP/1.1\r\n"));
            sleep(Duration::from_millis(40)).await;
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
                .await
                .unwrap();
        };
        let (_, _, result) = timeout(Duration::from_secs(3), async {
            tokio::join!(server, origin, test_proxy_with_target(&proxy, probe_target))
        })
        .await
        .unwrap();
        assert!(result.reachable, "{:?}", result.error);
        assert!(result.latency_ms.unwrap() >= 35);
        assert!(!result.stale);
    }
}
