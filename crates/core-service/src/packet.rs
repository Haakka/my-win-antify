use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FlowKey {
    pub local: SocketAddrV4,
    pub remote: SocketAddrV4,
}

impl FlowKey {
    pub fn reversed(self) -> Self {
        Self {
            local: self.remote,
            remote: self.local,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FlowKeyV6 {
    pub local: SocketAddrV6,
    pub remote: SocketAddrV6,
}

impl FlowKeyV6 {
    pub fn reversed(self) -> Self {
        Self {
            local: self.remote,
            remote: self.local,
        }
    }
}

const FIN: u8 = 0x01;
const SYN: u8 = 0x02;
const RST: u8 = 0x04;
const ACK: u8 = 0x10;

pub(crate) struct Packet {
    pub key: FlowKey,
    tcp: usize,
    pub payload_len: usize,
    pub seq: u32,
    ack: u32,
    pub flags: u8,
}

impl Packet {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 || bytes[0] >> 4 != 4 || bytes[9] != 6 {
            return None;
        }
        let tcp = usize::from(bytes[0] & 15) * 4;
        let total = usize::from(word(bytes, 2));
        if tcp < 20 || total != bytes.len() || total < tcp + 20 || word(bytes, 6) & 0x3fff != 0 {
            return None;
        }
        let header = usize::from(bytes[tcp + 12] >> 4) * 4;
        if header < 20 || tcp + header > total {
            return None;
        }
        Some(Self {
            key: FlowKey {
                local: SocketAddrV4::new(Ipv4Addr::from(octets(bytes, 12)), word(bytes, tcp)),
                remote: SocketAddrV4::new(Ipv4Addr::from(octets(bytes, 16)), word(bytes, tcp + 2)),
            },
            tcp,
            payload_len: total - tcp - header,
            seq: u32::from_be_bytes(octets(bytes, tcp + 4)),
            ack: u32::from_be_bytes(octets(bytes, tcp + 8)),
            flags: bytes[tcp + 13],
        })
    }

    pub fn initial_syn(&self) -> bool {
        self.flags & (SYN | ACK | RST) == SYN
    }

    pub fn reset(&self) -> bool {
        self.flags & RST != 0
    }

    pub fn forward(&self, bytes: &mut [u8], relay_port: u16) {
        let mut key = self.key.reversed();
        key.local.set_port(self.key.local.port());
        key.remote.set_port(relay_port);
        rewrite(bytes, self.tcp, key);
    }

    pub fn reverse(&self, bytes: &mut [u8], original: FlowKey) {
        rewrite(bytes, self.tcp, original.reversed());
    }

    pub fn block_reset(&self) -> [u8; 40] {
        let mut bytes = [0; 40];
        bytes[0] = 0x45;
        bytes[2..4].copy_from_slice(&40u16.to_be_bytes());
        bytes[8] = 64;
        bytes[9] = 6;
        bytes[32] = 5 << 4;
        bytes[33] = RST | ACK;
        rewrite(&mut bytes, 20, self.key.reversed());
        let (seq, ack) = reset_numbers(self.seq, self.ack, self.flags, self.payload_len);
        bytes[24..28].copy_from_slice(&seq.to_be_bytes());
        bytes[28..32].copy_from_slice(&ack.to_be_bytes());
        bytes
    }
}

pub(crate) struct PacketV6 {
    pub key: FlowKeyV6,
    payload_len: usize,
    seq: u32,
    ack: u32,
    flags: u8,
}

impl PacketV6 {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 60 || bytes[0] >> 4 != 6 || 40 + usize::from(word(bytes, 4)) != bytes.len()
        {
            return None;
        }
        let mut tcp = 40;
        let mut next = bytes[6];
        while matches!(next, 0 | 43 | 60) {
            let extension = bytes.get(tcp..tcp + 2)?;
            next = extension[0];
            tcp += (usize::from(extension[1]) + 1) * 8;
        }
        // Only TCP and the extension headers above are supported, never fragments.
        if next != 6 || tcp + 20 > bytes.len() {
            return None;
        }
        let header = usize::from(bytes[tcp + 12] >> 4) * 4;
        if header < 20 || tcp + header > bytes.len() {
            return None;
        }
        Some(Self {
            key: FlowKeyV6 {
                local: SocketAddrV6::new(
                    Ipv6Addr::from(octets16(bytes, 8)),
                    word(bytes, tcp),
                    0,
                    0,
                ),
                remote: SocketAddrV6::new(
                    Ipv6Addr::from(octets16(bytes, 24)),
                    word(bytes, tcp + 2),
                    0,
                    0,
                ),
            },
            payload_len: bytes.len() - tcp - header,
            seq: u32::from_be_bytes(octets(bytes, tcp + 4)),
            ack: u32::from_be_bytes(octets(bytes, tcp + 8)),
            flags: bytes[tcp + 13],
        })
    }

    pub fn initial_syn(&self) -> bool {
        !self.reset() && self.flags & (SYN | ACK) == SYN
    }

    pub fn reset(&self) -> bool {
        self.flags & RST != 0
    }

    pub fn block_reset(&self) -> [u8; 60] {
        let mut bytes = [0; 60];
        bytes[0] = 0x60;
        bytes[4..6].copy_from_slice(&20u16.to_be_bytes());
        bytes[6..8].copy_from_slice(&[6, 64]);
        let key = self.key.reversed();
        bytes[8..24].copy_from_slice(&key.local.ip().octets());
        bytes[24..40].copy_from_slice(&key.remote.ip().octets());
        bytes[40..42].copy_from_slice(&key.local.port().to_be_bytes());
        bytes[42..44].copy_from_slice(&key.remote.port().to_be_bytes());
        let (seq, ack) = reset_numbers(self.seq, self.ack, self.flags, self.payload_len);
        bytes[44..48].copy_from_slice(&seq.to_be_bytes());
        bytes[48..52].copy_from_slice(&ack.to_be_bytes());
        bytes[52] = 5 << 4;
        bytes[53] = RST | ACK;
        bytes
    }
}

fn reset_numbers(seq: u32, ack: u32, flags: u8, payload_len: usize) -> (u32, u32) {
    let reset_seq = if flags & ACK != 0 { ack } else { 0 };
    let reset_ack = seq
        .wrapping_add(payload_len as u32)
        .wrapping_add(u32::from(flags & SYN != 0))
        .wrapping_add(u32::from(flags & FIN != 0));
    (reset_seq, reset_ack)
}

fn octets16(bytes: &[u8], start: usize) -> [u8; 16] {
    bytes[start..start + 16].try_into().unwrap()
}

fn rewrite(bytes: &mut [u8], tcp: usize, key: FlowKey) {
    bytes[12..16].copy_from_slice(&key.local.ip().octets());
    bytes[16..20].copy_from_slice(&key.remote.ip().octets());
    bytes[tcp..tcp + 2].copy_from_slice(&key.local.port().to_be_bytes());
    bytes[tcp + 2..tcp + 4].copy_from_slice(&key.remote.port().to_be_bytes());
}

fn word(bytes: &[u8], start: usize) -> u16 {
    u16::from_be_bytes(bytes[start..start + 2].try_into().unwrap())
}

fn octets(bytes: &[u8], start: usize) -> [u8; 4] {
    bytes[start..start + 4].try_into().unwrap()
}

#[cfg(test)]
pub(crate) fn checksums(bytes: &mut [u8]) {
    let tcp = Packet::parse(bytes).unwrap().tcp;
    bytes[10..12].fill(0);
    bytes[tcp + 16..tcp + 18].fill(0);
    let ip = complement(sum(&bytes[..tcp]));
    let transport =
        complement(sum(&bytes[12..20]) + 6 + (bytes.len() - tcp) as u32 + sum(&bytes[tcp..]));
    bytes[10..12].copy_from_slice(&ip.to_be_bytes());
    bytes[tcp + 16..tcp + 18].copy_from_slice(&transport.to_be_bytes());
}

#[cfg(test)]
fn sum(bytes: &[u8]) -> u32 {
    bytes
        .chunks(2)
        .map(|w| u32::from(u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)])))
        .sum()
}

#[cfg(test)]
fn complement(mut value: u32) -> u16 {
    while value > 0xffff {
        value = (value & 0xffff) + (value >> 16);
    }
    !(value as u16)
}

#[cfg(test)]
pub(crate) fn fixture(flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 44 + payload.len()];
    let len = bytes.len() as u16;
    bytes[0] = 0x46; // Includes IPv4 header options.
    bytes[2..4].copy_from_slice(&len.to_be_bytes());
    bytes[8..10].copy_from_slice(&[64, 6]);
    bytes[36] = 5 << 4;
    bytes[37] = flags;
    bytes[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
    bytes[32..36].copy_from_slice(&1234u32.to_be_bytes());
    bytes[44..].copy_from_slice(payload);
    rewrite(
        &mut bytes,
        24,
        FlowKey {
            local: "192.0.2.1:55000".parse().unwrap(),
            remote: "203.0.113.12:443".parse().unwrap(),
        },
    );
    bytes
}

#[cfg(test)]
pub(crate) fn fixture_v6(headers: &[u8], flags: u8, payload: &[u8]) -> Vec<u8> {
    let tcp = 40 + headers.len() * 8;
    let mut bytes = vec![0; tcp + 20 + payload.len()];
    bytes[0] = 0x60;
    let length = (bytes.len() - 40) as u16;
    bytes[4..6].copy_from_slice(&length.to_be_bytes());
    bytes[6] = headers.first().copied().unwrap_or(6);
    bytes[7] = 64;
    bytes[8..24].copy_from_slice(&"2001:db8::1".parse::<Ipv6Addr>().unwrap().octets());
    bytes[24..40].copy_from_slice(&"2001:db8::2".parse::<Ipv6Addr>().unwrap().octets());
    for (i, &next) in headers
        .iter()
        .skip(1)
        .chain([&6])
        .take(headers.len())
        .enumerate()
    {
        bytes[40 + i * 8] = next;
    }
    bytes[tcp..tcp + 2].copy_from_slice(&55000u16.to_be_bytes());
    bytes[tcp + 2..tcp + 4].copy_from_slice(&443u16.to_be_bytes());
    bytes[tcp + 4..tcp + 8].copy_from_slice(&u32::MAX.to_be_bytes());
    bytes[tcp + 8..tcp + 12].copy_from_slice(&1234u32.to_be_bytes());
    bytes[tcp + 12] = 5 << 4;
    bytes[tcp + 13] = flags;
    bytes[tcp + 20..].copy_from_slice(payload);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_checksums(bytes: &[u8]) {
        let tcp = Packet::parse(bytes).unwrap().tcp;
        assert_eq!(complement(sum(&bytes[..tcp])), 0);
        assert_eq!(
            complement(sum(&bytes[12..20]) + 6 + (bytes.len() - tcp) as u32 + sum(&bytes[tcp..])),
            0
        );
    }

    #[test]
    fn reflect_both_directions_and_preserve_payload() {
        let mut forward = fixture(ACK, b"abc");
        let original = Packet::parse(&forward).unwrap();
        original.forward(&mut forward, 61000);
        let reflected = Packet::parse(&forward).unwrap();
        assert_eq!(reflected.key.local, "203.0.113.12:55000".parse().unwrap());
        assert_eq!(reflected.key.remote, "192.0.2.1:61000".parse().unwrap());
        assert_eq!(&forward[44..], b"abc");
        checksums(&mut forward);
        verify_checksums(&forward);

        let back = reflected.key.reversed();
        rewrite(&mut forward, reflected.tcp, back);
        Packet::parse(&forward)
            .unwrap()
            .reverse(&mut forward, original.key);
        assert_eq!(
            Packet::parse(&forward).unwrap().key,
            original.key.reversed()
        );
        checksums(&mut forward);
        verify_checksums(&forward);
    }

    #[test]
    fn block_reset_arithmetic_and_checksums() {
        for (flags, payload, seq, ack) in [(SYN, &b""[..], 0, 0), (ACK | FIN, &b"abc"[..], 1234, 3)]
        {
            let input = Packet::parse(&fixture(flags, payload)).unwrap();
            let mut reset = input.block_reset();
            let packet = Packet::parse(&reset).unwrap();
            assert_eq!(packet.flags, RST | ACK);
            assert_eq!(packet.key, input.key.reversed());
            assert_eq!((packet.seq, packet.ack, packet.payload_len), (seq, ack, 0));
            checksums(&mut reset);
            verify_checksums(&reset);
        }
    }

    #[test]
    fn parses_ipv6_tcp_with_and_without_extension_headers() {
        for headers in [&[][..], &[0][..], &[0, 43, 60][..]] {
            let bytes = fixture_v6(headers, SYN, b"hello");
            let packet = PacketV6::parse(&bytes).unwrap();
            assert!(packet.initial_syn());
            assert_eq!(packet.key.local, "[2001:db8::1]:55000".parse().unwrap());
            assert_eq!(packet.key.remote, "[2001:db8::2]:443".parse().unwrap());
            assert_eq!(
                (packet.seq, packet.ack, packet.payload_len),
                (u32::MAX, 1234, 5)
            );
        }
    }

    #[test]
    fn ipv6_rejects_fragments_and_invalid_lengths() {
        assert!(PacketV6::parse(&fixture_v6(&[44], SYN, &[])).is_none());
        assert!(PacketV6::parse(&fixture_v6(&[0, 44], SYN, &[])).is_none());
        let mut bytes = fixture_v6(&[60], SYN, &[]);
        assert!(PacketV6::parse(&bytes[..bytes.len() - 1]).is_none());
        bytes[41] = 8;
        assert!(PacketV6::parse(&bytes).is_none());
        let mut bytes = fixture_v6(&[], SYN, &[]);
        bytes[52] = 4 << 4;
        assert!(PacketV6::parse(&bytes).is_none());
    }

    #[test]
    fn ipv6_reset_reverses_endpoints_and_acknowledges_syn_and_payload() {
        for (flags, payload, seq, ack) in [(SYN, &b""[..], 0, 0), (ACK | FIN, &b"abc"[..], 1234, 3)]
        {
            let input = PacketV6::parse(&fixture_v6(&[0, 60], flags, payload)).unwrap();
            let bytes = input.block_reset();
            let reset = PacketV6::parse(&bytes).unwrap();
            assert_eq!(reset.key, input.key.reversed());
            assert_eq!(reset.flags, RST | ACK);
            assert!(reset.reset());
            assert!(!reset.initial_syn());
            assert_eq!((reset.seq, reset.ack, reset.payload_len), (seq, ack, 0));
            assert_eq!(&bytes[..8], &[0x60, 0, 0, 0, 0, 20, 6, 64]);
        }
    }

    #[test]
    fn rejects_fragments_and_invalid_lengths() {
        let mut bytes = fixture(SYN, &[]);
        assert!(Packet::parse(&bytes[..43]).is_none());
        bytes[6] = 0x20;
        assert!(Packet::parse(&bytes).is_none());
        bytes[6] = 0;
        bytes[36] = 15 << 4;
        assert!(Packet::parse(&bytes).is_none());
    }
}
