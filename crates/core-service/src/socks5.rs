use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::{Duration, Instant},
};

use shared::{ProxyConfig, ProxyProtocol, ProxyTestResult};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{lookup_host, TcpSocket, TcpStream},
    sync::OnceCell,
    time::timeout,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);

async fn handshake(proxy: &ProxyConfig) -> io::Result<TcpStream> {
    if proxy.protocol != ProxyProtocol::Socks5 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "M1 supports SOCKS5 proxies",
        ));
    }
    let host = proxy
        .host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&proxy.host);
    let mut stream = connect_proxy(host, proxy.port, proxy.interface_name.as_deref()).await?;
    stream.set_nodelay(true)?;
    let method = if proxy.auth.is_some() { 2 } else { 0 };
    stream.write_all(&[5, 1, method]).await?;
    let mut reply = [0; 2];
    stream.read_exact(&mut reply).await?;
    if reply != [5, method] {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SOCKS5 proxy rejected the authentication method",
        ));
    }
    if let Some(auth) = &proxy.auth {
        let username = auth.username.as_bytes();
        let password = auth.password.as_bytes();
        if !(1..=255).contains(&username.len()) || !(1..=255).contains(&password.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 credentials must be 1–255 bytes each",
            ));
        }
        let mut credentials = Vec::with_capacity(username.len() + password.len() + 3);
        credentials.extend([1, username.len() as u8]);
        credentials.extend(username);
        credentials.push(password.len() as u8);
        credentials.extend(password);
        stream.write_all(&credentials).await?;
        stream.read_exact(&mut reply).await?;
        if reply != [1, 0] {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SOCKS5 username/password authentication failed",
            ));
        }
    }
    Ok(stream)
}

async fn connect_proxy(host: &str, port: u16, interface: Option<&str>) -> io::Result<TcpStream> {
    let Some(interface) = interface else {
        return TcpStream::connect((host, port)).await;
    };

    let source = OnceCell::new();
    let mut last_error = io::Error::new(
        io::ErrorKind::AddrNotAvailable,
        "Proxy host resolved to no addresses",
    );
    let mut addresses: Vec<_> = lookup_host((host, port))
        .await?
        .map(|remote| match remote {
            SocketAddr::V6(v6) => v6
                .ip()
                .to_ipv4_mapped()
                .map_or(remote, |ip| SocketAddrV4::new(ip, v6.port()).into()),
            _ => remote,
        })
        .collect();
    // The selected interface is IPv4. Try its family first for dual-stack names.
    addresses.sort_by_key(SocketAddr::is_ipv6);
    for remote in addresses {
        let result = if remote.ip().is_loopback() {
            tracing::debug!(%remote, "Skipping proxy interface binding for loopback address");
            TcpStream::connect(remote).await
        } else if let SocketAddr::V4(remote) = remote {
            let source = source.get_or_init(|| interface_address(interface)).await;
            match source {
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
    let interfaces = tokio::task::spawn_blocking(crate::network_interfaces::list)
        .await
        .map_err(io::Error::other)??;
    let interface = interfaces
        .into_iter()
        .find(|interface| interface.name == name)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("Selected proxy interface {name:?} has no IPv4 address"),
            )
        })?;
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

pub(crate) async fn connect(
    proxy: &ProxyConfig,
    destination: SocketAddrV4,
    domain: Option<&str>,
) -> io::Result<TcpStream> {
    timeout(HANDSHAKE_TIMEOUT, async {
        let mut stream = handshake(proxy).await?;
        let mut request = vec![5, 1, 0];
        if let Some(domain) = domain.filter(|name| (1..=255).contains(&name.len())) {
            request.extend([3, domain.len() as u8]);
            request.extend(domain.as_bytes());
        } else {
            request.push(1);
            request.extend(destination.ip().octets());
        }
        request.extend(destination.port().to_be_bytes());
        stream.write_all(&request).await?;

        let mut header = [0; 4];
        stream.read_exact(&mut header).await?;
        if header[0] != 5 || header[2] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid SOCKS5 CONNECT reply",
            ));
        }
        if header[1] != 0 {
            return Err(io::Error::other(format!(
                "SOCKS5 CONNECT failed (reply 0x{:02x})",
                header[1]
            )));
        }
        let address_len = match header[3] {
            1 => 4,
            3 => usize::from(stream.read_u8().await?),
            4 => 16,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid SOCKS5 reply address type",
                ))
            }
        };
        let mut bound = [0; 257];
        stream.read_exact(&mut bound[..address_len + 2]).await?;
        Ok(stream)
    })
    .await
    .map_err(|_| timed_out())?
}

pub(crate) async fn test_proxy(proxy: &ProxyConfig) -> ProxyTestResult {
    let start = Instant::now();
    let result = timeout(HANDSHAKE_TIMEOUT, handshake(proxy))
        .await
        .map_err(|_| timed_out())
        .and_then(|result| result);
    ProxyTestResult {
        proxy_id: proxy.id.clone(),
        reachable: result.is_ok(),
        latency_ms: result.is_ok().then(|| start.elapsed().as_millis() as u32),
        error: result.err().map(|err| err.to_string()),
        checked_at_ms: crate::now_ms(),
    }
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "SOCKS5 connection or handshake timed out",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn configured_interface_is_ignored_for_resolved_loopback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        for host in ["127.0.0.1", "localhost", "::ffff:127.0.0.1"] {
            let stream = timeout(
                Duration::from_secs(2),
                connect_proxy(host, port, Some("missing adapter")),
            )
            .await
            .unwrap_or_else(|error| panic!("{host}: {error}"))
            .unwrap_or_else(|error| panic!("{host}: {error}"));
            let (_accepted, peer) = listener.accept().await.unwrap();
            assert!(stream.peer_addr().unwrap().ip().is_loopback());
            assert!(peer.ip().is_loopback());
        }
    }

    #[tokio::test]
    async fn no_auth_ipv4_connect_and_probe() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = ProxyConfig {
            id: "mock".into(),
            name: "Mock".into(),
            protocol: ProxyProtocol::Socks5,
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            auth: None,
            interface_name: None,
            allow_lan: false,
            udp_relay: false,
            fallback_direct: false,
            enabled: true,
        };
        let server = async {
            for probe in [false, true] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut greeting = [0; 3];
                stream.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                stream.write_all(&[5, 0]).await.unwrap();
                if !probe {
                    let mut request = [0; 10];
                    stream.read_exact(&mut request).await.unwrap();
                    assert_eq!(request, [5, 1, 0, 1, 203, 0, 113, 7, 1, 187]);
                    stream
                        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                        .await
                        .unwrap();
                }
            }
        };
        let client = async {
            connect(&proxy, "203.0.113.7:443".parse().unwrap(), None)
                .await
                .unwrap();
            let result = test_proxy(&proxy).await;
            assert!(result.reachable);
            assert!(result.latency_ms.is_some());
        };
        timeout(Duration::from_secs(2), async {
            tokio::join!(server, client)
        })
        .await
        .unwrap();
    }
}
