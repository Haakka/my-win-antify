use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4},
};

use shared::ProxyConfig;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{lookup_host, TcpStream},
    time::timeout,
};

use crate::{
    sniffer,
    upstream::{self, UpstreamError, CONNECT_TIMEOUT},
};

async fn handshake(proxy: &ProxyConfig) -> Result<TcpStream, UpstreamError> {
    let mut stream = upstream::connect_proxy(proxy).await?;
    stream.set_nodelay(true)?;
    let method = if proxy.auth.is_some() { 2 } else { 0 };
    stream.write_all(&[5, 1, method]).await?;
    let mut reply = [0; 2];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 5 {
        return Err(UpstreamError::protocol("Invalid SOCKS5 greeting version"));
    }
    if reply[1] != method {
        return Err(authentication_error(
            "SOCKS5 proxy rejected the authentication method",
        ));
    }
    if let Some(auth) = &proxy.auth {
        let username = auth.username.as_bytes();
        let password = auth.password.as_bytes();
        if !(1..=255).contains(&username.len()) || !(1..=255).contains(&password.len()) {
            return Err(UpstreamError::protocol(
                "SOCKS5 credentials must be 1-255 bytes each",
            ));
        }
        let mut credentials = Vec::with_capacity(username.len() + password.len() + 3);
        credentials.extend([1, username.len() as u8]);
        credentials.extend(username);
        credentials.push(password.len() as u8);
        credentials.extend(password);
        stream.write_all(&credentials).await?;
        stream.read_exact(&mut reply).await?;
        if reply[0] != 1 {
            return Err(UpstreamError::protocol(
                "Invalid SOCKS5 authentication version",
            ));
        }
        if reply[1] != 0 {
            return Err(authentication_error(
                "SOCKS5 username/password authentication failed",
            ));
        }
    }
    Ok(stream)
}

fn authentication_error(message: &str) -> UpstreamError {
    UpstreamError::proxy(
        "auth_failed",
        io::Error::new(io::ErrorKind::PermissionDenied, message),
    )
}

pub(crate) async fn connect(
    proxy: &ProxyConfig,
    destination: SocketAddrV4,
    domain: Option<&str>,
) -> Result<TcpStream, UpstreamError> {
    let mut stream = handshake(proxy).await?;
    let mut request = vec![5, 1, 0];
    if let Some(domain) = domain.and_then(sniffer::hostname) {
        request.extend([3, domain.len() as u8]);
        request.extend(domain.as_bytes());
    } else {
        request.push(1);
        request.extend(destination.ip().octets());
    }
    request.extend(destination.port().to_be_bytes());
    stream.write_all(&request).await?;
    command_reply(&mut stream, false).await?;
    Ok(stream)
}

pub(crate) async fn associate(
    proxy: &ProxyConfig,
) -> Result<(TcpStream, SocketAddr), UpstreamError> {
    timeout(CONNECT_TIMEOUT, async {
        let mut stream = handshake(proxy).await?;
        // The client UDP endpoint is assigned when the relay address family is known.
        stream.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        let reply = command_reply(&mut stream, true).await?;
        let peer = stream.peer_addr()?;
        let endpoint = match reply {
            BoundAddress::Ip(ip, port) => {
                SocketAddr::new(if ip.is_unspecified() { peer.ip() } else { ip }, port)
            }
            BoundAddress::Domain(host, port) => lookup_host((host, port))
                .await?
                .min_by_key(|addr| addr.is_ipv4() != peer.is_ipv4())
                .ok_or_else(|| {
                    UpstreamError::protocol("SOCKS5 UDP relay host resolved to no addresses")
                })?,
        };
        if endpoint.port() == 0 {
            return Err(UpstreamError::protocol(
                "SOCKS5 proxy returned UDP relay port zero",
            ));
        }
        Ok((stream, endpoint))
    })
    .await
    .map_err(|_| UpstreamError::timeout("SOCKS5 UDP ASSOCIATE timed out"))?
}

enum BoundAddress {
    Ip(IpAddr, u16),
    Domain(String, u16),
}

async fn command_reply(stream: &mut TcpStream, udp: bool) -> Result<BoundAddress, UpstreamError> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[2] != 0 {
        return Err(UpstreamError::protocol("Invalid SOCKS5 command reply"));
    }
    if header[1] != 0 {
        let message = format!(
            "SOCKS5 {} failed (reply 0x{:02x})",
            if udp { "UDP ASSOCIATE" } else { "CONNECT" },
            header[1]
        );
        if udp {
            return Err(UpstreamError::proxy(
                "protocol_error",
                io::Error::new(io::ErrorKind::Unsupported, message),
            ));
        }
        if matches!(header[1], 3..=5) {
            return Err(UpstreamError::destination(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                message,
            )));
        }
        return Err(UpstreamError::protocol(message));
    }
    match header[3] {
        1 => {
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await?;
            Ok(BoundAddress::Ip(
                Ipv4Addr::from(bytes).into(),
                stream.read_u16().await?,
            ))
        }
        3 => {
            let len = usize::from(stream.read_u8().await?);
            let mut bytes = vec![0; len];
            stream.read_exact(&mut bytes).await?;
            let host = std::str::from_utf8(&bytes)
                .ok()
                .and_then(sniffer::hostname)
                .ok_or_else(|| UpstreamError::protocol("Invalid domain in SOCKS5 reply"))?;
            Ok(BoundAddress::Domain(host, stream.read_u16().await?))
        }
        4 => {
            let mut bytes = [0; 16];
            stream.read_exact(&mut bytes).await?;
            Ok(BoundAddress::Ip(
                Ipv6Addr::from(bytes).into(),
                stream.read_u16().await?,
            ))
        }
        _ => Err(UpstreamError::protocol("Invalid SOCKS5 reply address type")),
    }
}
