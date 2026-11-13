//! DPI-desync примитивы (Цикл 7): fake-пакеты, TCP split, disorder.
//!
//! Перенос семантики техник оригинала (winws --dpi-desync=fake,split2,
//! disorder + fooling ttl/badsum/badseq) без использования winws.exe.
//!
//! Инвариант целостности соединений: фейковые пакеты обязаны быть
//! невалидными для удалённого хоста (низкий TTL / битая checksum /
//! невалидный SeqNum) — DPI видит их, конечный сервер отбрасывает.
//! Оригинальный пакет доходит неизменным (или фрагментами с корректными
//! checksums после пересчёта адаптером).

use super::packet::{L4Proto, PacketInfo};

/// Режим desync для одного пакета.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesyncMode {
    /// Пропустить оригинал без изменений.
    #[default]
    None,
    /// Fake перед оригиналом.
    Fake,
    /// Разрезать TCP payload на две части в позиции N.
    Split(usize),
    /// Fake, затем split.
    FakeSplit(usize),
    /// Split с отправкой частей в обратном порядке.
    Disorder(usize),
}

/// Приёмы «порчи» фейковых пакетов (fooling оригинала).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fooling {
    /// Установить TTL/HopLimit фейка (обычно 1-8).
    pub ttl: Option<u8>,
    /// Испортить TCP checksum фейка.
    pub bad_checksum: bool,
    /// Сдвинуть SeqNum фейка назад (невалидный для стека сервера).
    pub bad_seq: bool,
    /// Явный инкремент badseq (--dpi-desync-badseq-increment=N;
    /// перекрывает BAD_SEQ_SHIFT при заданном).
    pub badseq_increment: Option<i64>,
    /// Обнулить TCP Timestamps в фейке (--dpi-desync-fooling=ts).
    /// Требует включённых TCP timestamps на хосте (диагностика Цикла 6).
    pub corrupt_ts: bool,
}

/// Исходящий пакет от движка: данные + нужен ли пересчёт checksums.
#[derive(Debug)]
pub struct Outgoing {
    pub data: Vec<u8>,
    /// true — пересчитать checksums адаптером перед send;
    /// false — отправить как есть (фейк с намеренно битой checksum).
    pub fix_checksums: bool,
    /// true — фейковый пакет (для статистики fakes_sent).
    pub is_fake: bool,
}

impl Outgoing {
    fn fixed(data: Vec<u8>) -> Self {
        Self {
            data,
            fix_checksums: true,
            is_fake: false,
        }
    }
    fn fake(data: Vec<u8>, fix_checksums: bool) -> Self {
        Self {
            data,
            fix_checksums,
            is_fake: true,
        }
    }
}

/// Сдвиг SeqNum для bad_seq (семантика winws: невалидный seq вне окна).
const BAD_SEQ_SHIFT: u32 = 66_000;

/// Построить фейковый TCP-пакет на основе оригинала.
/// payload_override: заменить payload (например, чужой TLS ClientHello
/// из assets/payloads); длины заголовков корректируются.
/// Checksum НЕ пересчитывается здесь — либо портится (bad_checksum),
/// либо вычисляется адаптером (fix_checksums у результата).
pub fn build_fake_tcp(
    pkt: &[u8],
    info: &PacketInfo,
    fooling: &Fooling,
    payload_override: Option<&[u8]>,
) -> Option<Outgoing> {
    if info.proto != L4Proto::Tcp || pkt.len() < info.total_len {
        return None;
    }
    let mut fake = pkt[..info.total_len].to_vec();

    // Замена payload с корректировкой длин.
    if let Some(pl) = payload_override {
        fake.truncate(info.payload_off);
        fake.extend_from_slice(pl);
        fix_ip_total_len(&mut fake, info);
    }

    if let Some(ttl) = fooling.ttl {
        // IPv4: TTL — байт 8; IPv6: Hop Limit — байт 7.
        let off = if info.is_ipv6 { 7 } else { 8 };
        fake[off] = ttl;
    }
    if fooling.bad_seq || fooling.badseq_increment.is_some() {
        let shift = fooling.badseq_increment.unwrap_or(-(BAD_SEQ_SHIFT as i64));
        let seq = (info.tcp_seq? as i64).wrapping_add(shift) as u32;
        fake[info.l4_off + 4..info.l4_off + 8].copy_from_slice(&seq.to_be_bytes());
    }
    if fooling.corrupt_ts {
        corrupt_ts_option(&mut fake, info);
    }
    if fooling.bad_checksum {
        // Намеренно битая checksum: фейк не пройдёт стек сервера,
        // но DPI (без проверки checksum) увидит «валидный» пакет.
        let ck = info.l4_off + 16;
        fake[ck] = fake[ck].wrapping_add(0xBA);
        fake[ck + 1] = fake[ck + 1].wrapping_add(0xD0);
        return Some(Outgoing::fake(fake, false));
    }
    // TTL/seq-фейк: checksum обязана быть валидной — иначе DPI может
    // отбросить фейк раньше, чем увидит payload. Пересчёт — адаптером.
    Some(Outgoing::fake(fake, true))
}

/// Построить фейковый UDP-пакет (например, QUIC Initial из payload'ов).
pub fn build_fake_udp(
    pkt: &[u8],
    info: &PacketInfo,
    fooling: &Fooling,
    fake_payload: &[u8],
) -> Option<Outgoing> {
    if info.proto != L4Proto::Udp || pkt.len() < info.total_len {
        return None;
    }
    let mut fake = pkt[..info.payload_off].to_vec();
    fake.extend_from_slice(fake_payload);
    fix_ip_total_len(&mut fake, info);
    // UDP length — байты 4..6 UDP-заголовка.
    let udp_len = (8 + fake_payload.len()) as u16;
    fake[info.l4_off + 4..info.l4_off + 6].copy_from_slice(&udp_len.to_be_bytes());
    if let Some(ttl) = fooling.ttl {
        let off = if info.is_ipv6 { 7 } else { 8 };
        fake[off] = ttl;
    }
    Some(Outgoing::fake(fake, true))
}

/// Разрезать TCP-пакет на два сегмента в позиции pos от начала payload.
/// Checksums обеих частей пересчитываются адаптером.
pub fn split_tcp(pkt: &[u8], info: &PacketInfo, pos: usize) -> Option<(Outgoing, Outgoing)> {
    if info.proto != L4Proto::Tcp || pos == 0 || pos >= info.payload_len {
        return None;
    }
    let seq = info.tcp_seq?;

    // Часть 1: заголовки + payload[..pos].
    let mut part1 = pkt[..info.payload_off].to_vec();
    part1.extend_from_slice(&pkt[info.payload_off..info.payload_off + pos]);
    fix_ip_total_len(&mut part1, info);
    // Часть 1 не должна нести PSH (семантика незавершённого сегмента).
    part1[info.l4_off + 13] &= !0x08;

    // Часть 2: заголовки + payload[pos..], seq += pos.
    let mut part2 = pkt[..info.payload_off].to_vec();
    part2.extend_from_slice(&pkt[info.payload_off + pos..info.total_len]);
    fix_ip_total_len(&mut part2, info);
    let seq2 = seq.wrapping_add(pos as u32);
    part2[info.l4_off + 4..info.l4_off + 8].copy_from_slice(&seq2.to_be_bytes());

    Some((Outgoing::fixed(part1), Outgoing::fixed(part2)))
}

/// Полная обработка пакета по режиму: список исходящих пакетов
/// в порядке отправки.
pub fn process(
    pkt: &[u8],
    info: &PacketInfo,
    mode: DesyncMode,
    fooling: &Fooling,
    fake_payload: Option<&[u8]>,
) -> Vec<Outgoing> {
    let original = || Outgoing::fixed(pkt[..info.total_len].to_vec());
    match mode {
        DesyncMode::None => vec![original()],
        DesyncMode::Fake => {
            let mut out = Vec::new();
            match info.proto {
                L4Proto::Tcp => {
                    if let Some(f) = build_fake_tcp(pkt, info, fooling, fake_payload) {
                        out.push(f);
                    }
                }
                L4Proto::Udp => {
                    if let Some(pl) = fake_payload {
                        if let Some(f) = build_fake_udp(pkt, info, fooling, pl) {
                            out.push(f);
                        }
                    }
                }
                L4Proto::Other(_) => {}
            }
            out.push(original());
            out
        }
        DesyncMode::Split(pos) => match split_tcp(pkt, info, pos) {
            Some((a, b)) => vec![a, b],
            None => vec![original()],
        },
        DesyncMode::FakeSplit(pos) => {
            let mut out = Vec::new();
            if let Some(f) = build_fake_tcp(pkt, info, fooling, fake_payload) {
                out.push(f);
            }
            match split_tcp(pkt, info, pos) {
                Some((a, b)) => {
                    out.push(a);
                    out.push(b);
                }
                None => out.push(original()),
            }
            out
        }
        DesyncMode::Disorder(pos) => match split_tcp(pkt, info, pos) {
            // Обратный порядок: вторая часть уходит первой.
            Some((a, b)) => vec![b, a],
            None => vec![original()],
        },
    }
}

/// Обнулить TCP Timestamps option (kind 8) в заголовке пакета,
/// если она присутствует (fooling=ts оригинала). Без option — no-op.
pub fn corrupt_ts_option(pkt: &mut [u8], info: &PacketInfo) {
    if info.proto != L4Proto::Tcp {
        return;
    }
    let hdr_end = info.payload_off;
    let mut p = info.l4_off + 20; // начало TCP-опций
    while p < hdr_end {
        let kind = pkt[p];
        match kind {
            0 => break,  // EOL
            1 => p += 1, // NOP
            8 if p + 10 <= hdr_end => {
                // Timestamps: kind(1) len(1) TSval(4) TSecr(4)
                for b in &mut pkt[p + 2..p + 10] {
                    *b = 0;
                }
                return;
            }
            k if p + 2 <= hdr_end => {
                let len = pkt[p + 1] as usize;
                if len < 2 || p + len > hdr_end {
                    break;
                }
                let _ = k;
                p += len;
            }
            _ => break,
        }
    }
}

/// Обнулить поле IP ID (IPv4, --ip-id=zero оригинала).
pub fn zero_ip_id(pkt: &mut [u8], info: &PacketInfo) {
    if !info.is_ipv6 && pkt.len() >= 6 {
        pkt[4] = 0;
        pkt[5] = 0;
    }
}

/// Multisplit с перекрытием sequence (seqovl оригинала):
/// вторая часть начинается на pattern.len() байт РАНЬШЕ точки сплита,
/// а её перекрывающая часть заполнена паттерном (паттерн — обычно
/// чужой TLS ClientHello из assets/payloads). DPI собирает мусор,
/// стек сервера берёт данные первой части (перекрытие → предпочтение
/// первого сегмента).
pub fn split_tcp_seqovl(
    pkt: &[u8],
    info: &PacketInfo,
    pos: usize,
    pattern: &[u8],
) -> Option<(Outgoing, Outgoing)> {
    if info.proto != L4Proto::Tcp || pos == 0 || pos >= info.payload_len || pattern.is_empty() {
        return None;
    }
    let seq = info.tcp_seq?;
    let ovl = pattern.len();

    // Часть 1: как в обычном split (payload[..pos]).
    let (part1, _) = split_tcp(pkt, info, pos)?;

    // Часть 2: seq = seq + pos - ovl; payload = pattern + payload[pos..].
    let mut part2 = pkt[..info.payload_off].to_vec();
    part2.extend_from_slice(pattern);
    part2.extend_from_slice(&pkt[info.payload_off + pos..info.total_len]);
    fix_ip_total_len(&mut part2, info);
    let seq2 = (seq as i64 + pos as i64 - ovl as i64) as u32;
    part2[info.l4_off + 4..info.l4_off + 8].copy_from_slice(&seq2.to_be_bytes());

    Some((part1, Outgoing::fixed(part2)))
}

/// Мультипозиционный split: позиции применяются последовательно,
/// каждая — к оставшемуся хвосту payload. Возвращает сегменты в
/// прямом порядке (disorder-переворот — на вызывающей стороне).
pub fn split_tcp_multi(pkt: &[u8], info: &PacketInfo, positions: &[usize]) -> Vec<Outgoing> {
    let mut segments: Vec<(usize, usize)> = Vec::new(); // (start, end) в payload
    let mut cur = 0usize;
    for &pos in positions {
        if pos > cur && pos < info.payload_len {
            segments.push((cur, pos));
            cur = pos;
        }
    }
    if segments.is_empty() {
        return vec![];
    }
    segments.push((cur, info.payload_len));

    let seq = match info.tcp_seq {
        Some(s) => s,
        None => return vec![],
    };
    let mut out = Vec::with_capacity(segments.len());
    for (i, &(start, end)) in segments.iter().enumerate() {
        let mut part = pkt[..info.payload_off].to_vec();
        part.extend_from_slice(&pkt[info.payload_off + start..info.payload_off + end]);
        fix_ip_total_len(&mut part, info);
        let s = seq.wrapping_add(start as u32);
        part[info.l4_off + 4..info.l4_off + 8].copy_from_slice(&s.to_be_bytes());
        // PSH только у последнего сегмента
        if i + 1 < segments.len() {
            part[info.l4_off + 13] &= !0x08;
        }
        out.push(Outgoing::fixed(part));
    }
    out
}

/// Исправить поле длины IP-заголовка под фактический размер буфера.
fn fix_ip_total_len(pkt: &mut [u8], info: &PacketInfo) {
    if info.is_ipv6 {
        let plen = (pkt.len() - 40) as u16;
        pkt[4..6].copy_from_slice(&plen.to_be_bytes());
    } else {
        let total = (pkt.len()) as u16;
        pkt[2..4].copy_from_slice(&total.to_be_bytes());
    }
    // IP checksum инвалидируем — пересчёт адаптером.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TCP-пакет: 20 IP + 20 TCP + payload "0123456789".
    fn make_tcp(payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 40 + payload.len()];
        p[0] = 0x45;
        let total = (p.len() as u16).to_be_bytes();
        p[2] = total[0];
        p[3] = total[1];
        p[8] = 64;
        p[9] = 6;
        p[20..22].copy_from_slice(&5000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[24..28].copy_from_slice(&1000u32.to_be_bytes());
        p[32] = 0x50;
        p[33] = 0x18; // PSH+ACK
        p[40..].copy_from_slice(payload);
        p
    }

    fn info_of(p: &[u8]) -> PacketInfo {
        PacketInfo::parse(p).unwrap()
    }

    #[test]
    fn split_at_position() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        let (a, b) = split_tcp(&p, &info, 3).unwrap();
        assert_eq!(&a.data[40..], b"012");
        assert_eq!(&b.data[40..], b"3456789");
        // seq второй части = исходный + 3
        let seq2 = u32::from_be_bytes([b.data[24], b.data[25], b.data[26], b.data[27]]);
        assert_eq!(seq2, 1003);
        // длины IP заголовков
        assert_eq!(u16::from_be_bytes([a.data[2], a.data[3]]), 43);
        assert_eq!(u16::from_be_bytes([b.data[2], b.data[3]]), 47);
        // PSH снят у первой части
        assert_eq!(a.data[33] & 0x08, 0);
        assert_ne!(b.data[33] & 0x08, 0);
    }

    #[test]
    fn split_invalid_positions() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        assert!(split_tcp(&p, &info, 0).is_none());
        assert!(split_tcp(&p, &info, 10).is_none());
    }

    #[test]
    fn fake_with_ttl_and_badseq() {
        let p = make_tcp(b"\x16\x03\x01hello");
        let info = info_of(&p);
        let f = Fooling {
            ttl: Some(3),
            bad_checksum: false,
            bad_seq: true,
            ..Default::default()
        };
        let out = build_fake_tcp(&p, &info, &f, None).unwrap();
        assert!(out.fix_checksums);
        assert_eq!(out.data[8], 3); // TTL
        let seq = u32::from_be_bytes([out.data[24], out.data[25], out.data[26], out.data[27]]);
        assert_eq!(seq, 1000u32.wrapping_sub(BAD_SEQ_SHIFT));
    }

    #[test]
    fn fake_with_bad_checksum_not_fixed() {
        let p = make_tcp(b"data");
        let info = info_of(&p);
        let f = Fooling {
            ttl: None,
            bad_checksum: true,
            bad_seq: false,
            ..Default::default()
        };
        let out = build_fake_tcp(&p, &info, &f, None).unwrap();
        assert!(!out.fix_checksums);
        assert_ne!(&out.data[36..38], &[0, 0]); // checksum испорчена
    }

    #[test]
    fn fake_payload_override_adjusts_length() {
        let p = make_tcp(b"abc");
        let info = info_of(&p);
        let fake_hello = vec![0x16u8; 100];
        let out = build_fake_tcp(&p, &info, &Fooling::default(), Some(&fake_hello)).unwrap();
        assert_eq!(out.data.len(), 40 + 100);
        assert_eq!(u16::from_be_bytes([out.data[2], out.data[3]]), 140);
    }

    #[test]
    fn disorder_reverses_order() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        let out = process(
            &p,
            &info,
            DesyncMode::Disorder(4),
            &Fooling::default(),
            None,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(&out[0].data[40..], b"456789"); // вторая часть первой
        assert_eq!(&out[1].data[40..], b"0123");
    }

    #[test]
    fn udp_fake_quic() {
        let mut p = vec![0u8; 28 + 30];
        p[0] = 0x45;
        let total = (p.len() as u16).to_be_bytes();
        p[2] = total[0];
        p[3] = total[1];
        p[9] = 17;
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[28] = 0xC3;
        let info = PacketInfo::parse(&p).unwrap();
        let fake = vec![0xFFu8; 1200];
        let out = build_fake_udp(&p, &info, &Fooling::default(), &fake).unwrap();
        assert_eq!(out.data.len(), 28 + 1200);
        let udp_len = u16::from_be_bytes([out.data[24], out.data[25]]);
        assert_eq!(udp_len, 8 + 1200);
        assert_eq!(u16::from_be_bytes([out.data[2], out.data[3]]), 1228);
    }

    #[test]
    fn seqovl_overlaps_with_pattern() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        let pattern = b"XYZW".to_vec(); // 4 байта перекрытия
        let (a, b) = split_tcp_seqovl(&p, &info, 3, &pattern).unwrap();
        assert_eq!(&a.data[40..], b"012");
        // seq2 = 1000 + 3 - 4 = 999; payload = pattern + "3456789"
        let seq2 = u32::from_be_bytes([b.data[24], b.data[25], b.data[26], b.data[27]]);
        assert_eq!(seq2, 999);
        assert_eq!(&b.data[40..], b"XYZW3456789");
        assert!(!b.is_fake);
        assert!(b.fix_checksums);
    }

    #[test]
    fn multi_split_three_segments() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        let out = split_tcp_multi(&p, &info, &[2, 5]);
        assert_eq!(out.len(), 3);
        assert_eq!(&out[0].data[40..], b"01");
        assert_eq!(&out[1].data[40..], b"234");
        assert_eq!(&out[2].data[40..], b"56789");
        // seq: 1000, 1002, 1005
        let s1 = u32::from_be_bytes([
            out[1].data[24],
            out[1].data[25],
            out[1].data[26],
            out[1].data[27],
        ]);
        assert_eq!(s1, 1002);
        // PSH только у последнего
        assert_eq!(out[0].data[33] & 0x08, 0);
        assert_ne!(out[2].data[33] & 0x08, 0);
    }

    #[test]
    fn multi_split_unsorted_positions_ignored() {
        let p = make_tcp(b"0123456789");
        let info = info_of(&p);
        // Позиции применяются в заданном порядке (семантика winws):
        // после 7 позиция 2 уже позади → игнорируется; 50 за границей.
        let out = split_tcp_multi(&p, &info, &[7, 2, 50]);
        assert_eq!(out.len(), 2);
        assert_eq!(&out[0].data[40..], b"0123456");
        assert_eq!(&out[1].data[40..], b"789");
        // Отсортированные дают 3 сегмента.
        let out = split_tcp_multi(&p, &info, &[2, 7]);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn zero_ip_id_v4() {
        let mut p = make_tcp(b"x");
        p[4] = 0xAB;
        p[5] = 0xCD;
        let info = info_of(&p);
        zero_ip_id(&mut p, &info);
        assert_eq!(&p[4..6], &[0, 0]);
    }

    #[test]
    fn corrupt_ts_with_option() {
        // TCP с опцией Timestamps: 20 байт базовых + NOP,NOP,TS(10)
        let p = make_tcp(b"data");
        let hdr_len = 20 + 12;
        let total = (20 + hdr_len + 4) as u16;
        let mut q = vec![0u8; 20 + hdr_len + 4];
        q[..20].copy_from_slice(&p[..20]);
        q[2] = (total >> 8) as u8;
        q[3] = (total & 0xFF) as u8;
        q[20..40].copy_from_slice(&p[20..40]);
        q[32] = ((hdr_len / 4) as u8) << 4;
        // опции: NOP,NOP,TS,len=10,TSval=0x11223344,TSecr=0x55667788
        q[40] = 1;
        q[41] = 1;
        q[42] = 8;
        q[43] = 10;
        q[44..48].copy_from_slice(&0x11223344u32.to_be_bytes());
        q[48..52].copy_from_slice(&0x55667788u32.to_be_bytes());
        q[52..56].copy_from_slice(b"data");
        let info = PacketInfo::parse(&q).unwrap();
        assert_eq!(info.payload_len, 4);
        corrupt_ts_option(&mut q, &info);
        assert_eq!(&q[44..52], &[0u8; 8]);
    }
}
