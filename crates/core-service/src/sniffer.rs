use std::{net::IpAddr, time::Duration};

use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};

const PEEK_BYTES: usize = 8192;
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_millis(60);
const PEEK_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) async fn peek_domain(stream: &TcpStream) -> Option<String> {
    let mut bytes = vec![0; PEEK_BYTES];
    let mut len = timeout(FIRST_BYTE_TIMEOUT, stream.peek(&mut bytes))
        .await
        .ok()?
        .ok()?;
    timeout(PEEK_TIMEOUT, async {
        loop {
            let data = &bytes[..len];
            if data.is_empty() {
                return None;
            }
            let tls = data[0] == 22;
            let complete = if tls {
                data.get(3..5).is_some_and(|size| {
                    data.len() >= 5 + usize::from(u16::from_be_bytes([size[0], size[1]]))
                })
            } else {
                if !data[0].is_ascii_uppercase() {
                    return None;
                }
                data.windows(4).any(|bytes| bytes == b"\r\n\r\n")
            };
            if complete || len == PEEK_BYTES {
                return if tls { tls_sni(data) } else { http_host(data) };
            }
            // Peek leaves the same bytes readable while subsequent TCP segments arrive.
            sleep(Duration::from_millis(8)).await;
            len = stream.peek(&mut bytes).await.ok()?;
        }
    })
    .await
    .ok()
    .flatten()
}

fn tls_sni(mut bytes: &[u8]) -> Option<String> {
    let record = take(&mut bytes, 5)?;
    if record[0] != 22 || record[1] != 3 {
        return None;
    }
    let mut record = take(
        &mut bytes,
        u16::from_be_bytes([record[3], record[4]]) as usize,
    )?;
    let handshake = take(&mut record, 4)?;
    if handshake[0] != 1 {
        return None;
    }
    let mut hello = take(
        &mut record,
        u32::from_be_bytes([0, handshake[1], handshake[2], handshake[3]]) as usize,
    )?;
    take(&mut hello, 34)?; // Client version and random bytes.
    vector(&mut hello, 1)?; // Session ID.
    vector(&mut hello, 2)?; // Cipher suites.
    vector(&mut hello, 1)?; // Compression methods.
    let mut extensions = vector(&mut hello, 2)?;
    while !extensions.is_empty() {
        let kind = take(&mut extensions, 2)?;
        let mut extension = vector(&mut extensions, 2)?;
        if kind == [0, 0] {
            let mut names = vector(&mut extension, 2)?;
            while !names.is_empty() {
                let kind = take(&mut names, 1)?[0];
                let name = vector(&mut names, 2)?;
                if kind == 0 {
                    return hostname(std::str::from_utf8(name).ok()?);
                }
            }
        }
    }
    None
}

fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, rest) = bytes.split_at_checked(n)?;
    *bytes = rest;
    Some(head)
}

fn vector<'a>(bytes: &mut &'a [u8], width: usize) -> Option<&'a [u8]> {
    let len = take(bytes, width)?
        .iter()
        .fold(0, |len, &byte| len * 256 + usize::from(byte));
    take(bytes, len)
}

fn http_host(bytes: &[u8]) -> Option<String> {
    let end = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n")?;
    let mut headers = bytes[..end].split(|&byte| byte == b'\n');
    let request = std::str::from_utf8(headers.next()?)
        .ok()?
        .trim_end_matches('\r');
    let mut parts = request.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    if method.is_empty()
        || !method.bytes().all(|byte| byte.is_ascii_uppercase())
        || target.is_empty()
        || !matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1"))
        || parts.next().is_some()
    {
        return None;
    }
    let mut host = None;
    for header in headers {
        let Some(colon) = header.iter().position(|&b| b == b':') else {
            continue;
        };
        if header[..colon].eq_ignore_ascii_case(b"host") {
            if host.is_some() {
                return None;
            }
            let value = std::str::from_utf8(&header[colon + 1..]).ok()?.trim();
            let name = if let Some((name, port)) = value.rsplit_once(':') {
                port.parse::<u16>().ok()?;
                name
            } else {
                value
            };
            host = Some(hostname(name)?);
        }
    }
    host
}

pub(crate) fn hostname(name: &str) -> Option<String> {
    if name.len() > 253 || name.parse::<IpAddr>().is_ok() {
        return None;
    }
    let labels = name.strip_suffix('.').unwrap_or(name).split('.');
    labels
        .into_iter()
        .all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .then(|| name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_hello(extension_type: u8) -> Vec<u8> {
        with_sni_extension(Vec::new(), extension_type)
    }

    fn with_sni_extension(mut extensions: Vec<u8>, extension_type: u8) -> Vec<u8> {
        let name = b"WWW.Example.org";
        let mut extension = vec![0, (name.len() + 3) as u8, 0, 0, name.len() as u8];
        extension.extend(name);
        extensions.extend([0, extension_type, 0, extension.len() as u8]);
        extensions.extend(extension);
        let mut hello = vec![0; 34];
        hello[..2].copy_from_slice(&[3, 3]);
        hello.extend([0, 0, 2, 0x13, 1, 1, 0]);
        hello.extend((extensions.len() as u16).to_be_bytes());
        hello.extend(extensions);
        let mut record = vec![22, 3, 1];
        record.extend(((hello.len() + 4) as u16).to_be_bytes());
        record.push(1);
        record.extend(&(hello.len() as u32).to_be_bytes()[1..]);
        record.extend(hello);
        record
    }

    #[test]
    fn extracts_client_hello_sni_and_rejects_truncations() {
        let hello = client_hello(0);
        assert_eq!(tls_sni(&hello).as_deref(), Some("www.example.org"));
        for len in 0..hello.len() {
            assert!(tls_sni(&hello[..len]).is_none(), "truncated length {len}");
        }
        assert!(tls_sni(&client_hello(1)).is_none());
        assert!(tls_sni(b"get https://example.org").is_none());
    }

    #[tokio::test]
    async fn silent_client_releases_server_first_relay_quickly() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let start = std::time::Instant::now();
        assert!(peek_domain(&stream).await.is_none());
        assert!(start.elapsed() < Duration::from_millis(120));
    }

    #[tokio::test]
    async fn peeks_split_large_tls13_client_hello_without_consuming_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // TLS 1.3 supported_versions, a synthetic hybrid key share, and padding before SNI.
        let mut extensions = vec![0, 43, 0, 3, 2, 3, 4];
        let key_len = 1216u16;
        extensions.extend([0, 51]);
        extensions.extend((key_len + 6).to_be_bytes());
        extensions.extend((key_len + 4).to_be_bytes());
        extensions.extend(0x11ecu16.to_be_bytes());
        extensions.extend(key_len.to_be_bytes());
        extensions.resize(extensions.len() + usize::from(key_len), 0x41);
        let padding_len = 1600u16;
        extensions.extend([0, 21]);
        extensions.extend(padding_len.to_be_bytes());
        extensions.resize(extensions.len() + usize::from(padding_len), 0);
        let hello = with_sni_extension(extensions, 0);
        assert!(hello.len() > 2500);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        let write = async {
            client.write_all(&hello[..1300]).await.unwrap();
            sleep(Duration::from_millis(25)).await;
            client.write_all(&hello[1300..]).await.unwrap();
        };
        let read = async {
            assert_eq!(
                peek_domain(&server).await.as_deref(),
                Some("www.example.org")
            );
            let mut unconsumed = vec![0; hello.len()];
            server.read_exact(&mut unconsumed).await.unwrap();
            assert_eq!(unconsumed, hello);
        };
        timeout(Duration::from_secs(1), async { tokio::join!(write, read) })
            .await
            .unwrap();
    }

    #[test]
    fn extracts_case_insensitive_http_host_and_ignores_other_payloads() {
        let request = b"GET / HTTP/1.1\r\nhOsT: Example.org:8080\r\nX-Bytes: \xff\r\n\r\n";
        assert_eq!(http_host(request).as_deref(), Some("example.org"));
        for len in 0..request.len() {
            assert!(http_host(&request[..len]).is_none());
        }
        assert!(http_host(b"Host: example.org\r\n\r\n").is_none());
        assert!(http_host(b"GET / HTTP/1.1\r\nHost: one.org\r\nHost: two.org\r\n\r\n").is_none());
        assert!(http_host(b"GET / HTTP/1.1\r\nHost: 192.0.2.1\r\n\r\n").is_none());
    }
}
