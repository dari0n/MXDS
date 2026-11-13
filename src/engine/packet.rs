//! Чистый разбор сетевых пакетов (без FFI) — тестируемо без драйвера.
//!
//! Используется рабочим циклом движка для решений по профилю:
//! определение TCP/UDP, портов, смещений заголовков и payload.

use std::net::{Ipv4Addr, Ipv6Addr};

/// Протокол транспортного уровня.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L4Proto {
    Tcp,
    Udp,
    Other(u8),
}

/// Адресация пакета.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpAddr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

/// Сводная информация о пакете (offsets в буфере + ключевые поля).
#[derive(Debug, Clone, Copy)]
pub struct PacketInfo {
    pub is_ipv6: bool,
    /// Смещение IP-заголовка (всегда 0 на NETWORK-слое).
    pub ip_off: usize,
    /// Длина IP-заголовка (с опциями для IPv4).
    pub ip_hdr_len: usize,
    /// Смещение транспортного заголовка.
    pub l4_off: usize,
    /// Смещение полезной нагрузки (0, если не применимо).
    pub payload_off: usize,
    /// Длина полезной нагрузки.
    pub payload_len: usize,
    /// Полная длина пакета по IP-заголовку.
    pub total_len: usize,
    pub proto: L4Proto,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    /// Флаги TCP (байт 13 заголовка): FIN=1, SYN=2, RST=4, PSH=8, ACK=16.
    pub tcp_flags: Option<u8>,
    /// SeqNum TCP (host order), для split-операций.
    pub tcp_seq: Option<u32>,
    /// TTL (IPv4) / Hop Limit (IPv6).
    pub ttl: u8,
}

/// Контекст разбора L4 (параметры после IP-заголовка).
#[derive(Clone, Copy)]
struct L4Context {
    is_ipv6: bool,
    l4_off: usize,
    total_len: usize,
    proto: u8,
    src: IpAddr,
    dst: IpAddr,
    ttl: u8,
}

impl PacketInfo {
    /// Разобрать пакет NETWORK-слоя (начинается с IP-заголовка).
    /// None — пакет не IP или усечён.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 20 {
            return None;
        }
        match buf[0] >> 4 {
            4 => Self::parse_v4(buf),
            6 => Self::parse_v6(buf),
            _ => None,
        }
    }

    fn parse_v4(buf: &[u8]) -> Option<Self> {
        let ihl = ((buf[0] & 0x0F) as usize) * 4;
        if ihl < 20 || buf.len() < ihl {
            return None;
        }
        let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        if total_len < ihl || buf.len() < total_len {
            return None;
        }
        let proto = buf[9];
        let src = IpAddr::V4(Ipv4Addr::new(buf[12], buf[13], buf[14], buf[15]));
        let dst = IpAddr::V4(Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]));
        let ttl = buf[8];
        let ctx = L4Context {
            is_ipv6: false,
            l4_off: ihl,
            total_len,
            proto,
            src,
            dst,
            ttl,
        };
        Self::parse_l4(buf, &ctx)
    }

    fn parse_v6(buf: &[u8]) -> Option<Self> {
        if buf.len() < 40 {
            return None;
        }
        let payload_len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
        let total_len = 40 + payload_len;
        if buf.len() < total_len {
            return None;
        }
        let ttl = buf[7]; // Hop Limit
        let src = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&buf[8..24]).ok()?));
        let dst = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&buf[24..40]).ok()?));

        // Цепочка extension-заголовков: Hop-by-Hop(0), Routing(43),
        // Fragment(44), Destination(60). Fragment-заголовок — 8 байт.
        let mut next = buf[6];
        let mut off = 40usize;
        loop {
            match next {
                0 | 43 | 60 => {
                    if buf.len() < off + 2 {
                        return None;
                    }
                    let ext_len = ((buf[off + 1] as usize) + 1) * 8;
                    next = buf[off];
                    off += ext_len;
                    if off > total_len {
                        return None;
                    }
                }
                44 => {
                    // Фрагментированный пакет: обрабатываем только первый
                    // фрагмент (offset=0, M=0/1 — L4 виден в первом).
                    if buf.len() < off + 8 {
                        return None;
                    }
                    let frag = u16::from_be_bytes([buf[off + 2], buf[off + 3]]);
                    if frag & 0xFFF8 != 0 {
                        return None; // не первый фрагмент — пропускаем
                    }
                    next = buf[off];
                    off += 8;
                }
                _ => break,
            }
        }
        let ctx = L4Context {
            is_ipv6: true,
            l4_off: off,
            total_len,
            proto: next,
            src,
            dst,
            ttl,
        };
        Self::parse_l4(buf, &ctx)
    }

    /// Контекст разбора L4 (группировка параметров — clippy).
    fn parse_l4(buf: &[u8], ctx: &L4Context) -> Option<Self> {
        let (is_ipv6, l4_off, total_len, proto, src, dst, ttl) = (
            ctx.is_ipv6,
            ctx.l4_off,
            ctx.total_len,
            ctx.proto,
            ctx.src,
            ctx.dst,
            ctx.ttl,
        );
        let (l4proto, src_port, dst_port, payload_off, tcp_flags, tcp_seq) = match proto {
            6 => {
                if buf.len() < l4_off + 20 {
                    return None;
                }
                let sp = u16::from_be_bytes([buf[l4_off], buf[l4_off + 1]]);
                let dp = u16::from_be_bytes([buf[l4_off + 2], buf[l4_off + 3]]);
                let seq = u32::from_be_bytes([
                    buf[l4_off + 4],
                    buf[l4_off + 5],
                    buf[l4_off + 6],
                    buf[l4_off + 7],
                ]);
                let data_off = ((buf[l4_off + 12] >> 4) as usize) * 4;
                if data_off < 20 || buf.len() < l4_off + data_off {
                    return None;
                }
                (
                    L4Proto::Tcp,
                    sp,
                    dp,
                    l4_off + data_off,
                    Some(buf[l4_off + 13]),
                    Some(seq),
                )
            }
            17 => {
                if buf.len() < l4_off + 8 {
                    return None;
                }
                let sp = u16::from_be_bytes([buf[l4_off], buf[l4_off + 1]]);
                let dp = u16::from_be_bytes([buf[l4_off + 2], buf[l4_off + 3]]);
                (L4Proto::Udp, sp, dp, l4_off + 8, None, None)
            }
            other => (L4Proto::Other(other), 0, 0, l4_off, None, None),
        };
        let payload_len = total_len.saturating_sub(payload_off);
        Some(Self {
            is_ipv6,
            ip_off: 0,
            ip_hdr_len: l4_off,
            l4_off,
            payload_off,
            payload_len,
            total_len,
            proto: l4proto,
            src,
            dst,
            src_port,
            dst_port,
            tcp_flags,
            tcp_seq,
            ttl,
        })
    }

    /// TCP payload похож на TLS ClientHello (record 0x16, версия 0x03xx).
    pub fn is_tls_client_hello(&self, buf: &[u8]) -> bool {
        self.proto == L4Proto::Tcp
            && self.payload_len >= 3
            && buf[self.payload_off] == 0x16
            && buf[self.payload_off + 1] == 0x03
    }

    /// UDP payload похож на QUIC Initial (long header, биты 11xxxxxx).
    pub fn is_quic_initial(&self, buf: &[u8]) -> bool {
        self.proto == L4Proto::Udp
            && self.payload_len > 16
            && (buf[self.payload_off] & 0xC0) == 0xC0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Минимальный TCP-пакет IPv4 с payload "abc".
    fn tcp_packet() -> Vec<u8> {
        let mut p = vec![0u8; 20 + 20 + 3];
        p[0] = 0x45; // v4, ihl 5
        let total = (p.len() as u16).to_be_bytes();
        p[2] = total[0];
        p[3] = total[1];
        p[8] = 64; // TTL
        p[9] = 6; // TCP
        p[12..16].copy_from_slice(&[192, 168, 0, 2]);
        p[16..20].copy_from_slice(&[93, 184, 216, 34]);
        p[20..22].copy_from_slice(&12345u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[24..28].copy_from_slice(&0xAABBCCDDu32.to_be_bytes());
        p[32] = 0x50; // data offset 5
        p[33] = 0x18; // PSH+ACK
        p[40..43].copy_from_slice(&[0x16, 0x03, 0x01]); // TLS record
        p
    }

    #[test]
    fn parse_ipv4_tcp() {
        let p = tcp_packet();
        let info = PacketInfo::parse(&p).unwrap();
        assert!(!info.is_ipv6);
        assert_eq!(info.proto, L4Proto::Tcp);
        assert_eq!(info.src_port, 12345);
        assert_eq!(info.dst_port, 443);
        assert_eq!(info.tcp_flags, Some(0x18));
        assert_eq!(info.tcp_seq, Some(0xAABBCCDD));
        assert_eq!(info.ttl, 64);
        assert_eq!(info.payload_off, 40);
        assert_eq!(info.payload_len, 3);
        assert!(info.is_tls_client_hello(&p));
        assert!(!info.is_quic_initial(&p));
    }

    #[test]
    fn parse_ipv4_udp() {
        let mut p = vec![0u8; 20 + 8 + 20];
        p[0] = 0x45;
        let total = (p.len() as u16).to_be_bytes();
        p[2] = total[0];
        p[3] = total[1];
        p[9] = 17;
        p[20..22].copy_from_slice(&60000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[28] = 0xC3; // QUIC long header
        let info = PacketInfo::parse(&p).unwrap();
        assert_eq!(info.proto, L4Proto::Udp);
        assert_eq!(info.dst_port, 443);
        assert_eq!(info.payload_off, 28);
        assert_eq!(info.payload_len, 20);
        assert!(info.is_quic_initial(&p));
    }

    #[test]
    fn parse_ipv6_tcp() {
        let mut p = vec![0u8; 40 + 20 + 1];
        p[0] = 0x60;
        let plen = 21u16.to_be_bytes();
        p[4] = plen[0];
        p[5] = plen[1];
        p[6] = 6; // next = TCP
        p[7] = 64;
        p[40..42].copy_from_slice(&1111u16.to_be_bytes());
        p[42..44].copy_from_slice(&80u16.to_be_bytes());
        p[52] = 0x50;
        p[53] = 0x02; // SYN
        let info = PacketInfo::parse(&p).unwrap();
        assert!(info.is_ipv6);
        assert_eq!(info.proto, L4Proto::Tcp);
        assert_eq!(info.src_port, 1111);
        assert_eq!(info.dst_port, 80);
        assert_eq!(info.tcp_flags, Some(0x02));
    }

    #[test]
    fn truncated_rejected() {
        assert!(PacketInfo::parse(&[0x45, 0, 0]).is_none());
        let mut p = tcp_packet();
        p.truncate(30);
        assert!(PacketInfo::parse(&p).is_none());
    }
}
