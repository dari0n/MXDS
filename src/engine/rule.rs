//! Правила движка: матчинг пакетов по протоколу/портам/hostlist/ipset/L7
//! и применение desync-действия. EnginePlan — упорядоченный список правил;
//! первое совпавшее правило выигрывает (семантика winws).
//!
//! Матчеры списков передаются как замыкания — engine не зависит от
//! strategy/ (направление зависимости: strategy → engine).

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::desync::{self, Fooling, Outgoing};
use super::packet::{L4Proto, PacketInfo};
use super::sni;

/// Матчер домена (скомпилированный hostlist с исключениями).
pub type HostMatchFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;
/// Матчер IP (скомпилированный ipset с исключениями).
pub type IpMatchFn = Arc<dyn Fn(Ipv4Addr) -> bool + Send + Sync>;

/// Позиция сплита на стороне движка (вычисляемые разрешаются по пакету).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSplitPos {
    Absolute(usize),
    SniExtPlus(usize),
    MidSld,
}

/// Класс трафика для L7-фильтра правила.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficKind {
    Tls,
    Quic,
    Http,
    /// Discord voice / STUN (UDP с magic cookie STUN).
    DiscordStun,
    /// Нераспознанный протокол.
    Unknown,
}

/// Действие правила (что делать с совпавшим пакетом).
#[derive(Clone)]
pub struct RuleAction {
    /// Техники в порядке winws. Syndata не поддерживается движком —
    /// отфильтровывается валидацией Strategy Manager.
    pub techniques: Vec<RuleTechnique>,
    /// Повторы фейка (repeats=N).
    pub repeats: u32,
    pub split_pos: Vec<RuleSplitPos>,
    /// Паттерн seqovl (содержимое файла).
    pub seqovl_pattern: Option<Vec<u8>>,
    pub fooling: Fooling,
    /// Прекращение desync после N обработанных пакетов соединения.
    pub cutoff: Option<u32>,
    /// Применять к нераспознанным протоколам (--dpi-desync-any-protocol).
    pub any_protocol: bool,
    // Фейковые payload'ы по классам трафика.
    pub fake_tls: Option<Vec<u8>>,
    pub fake_quic: Option<Vec<u8>>,
    pub fake_http: Option<Vec<u8>>,
    pub fake_discord: Option<Vec<u8>>,
    pub fake_unknown_udp: Option<Vec<u8>>,
    /// --ip-id=zero.
    pub ip_id_zero: bool,
}

impl Default for RuleAction {
    fn default() -> Self {
        Self {
            techniques: vec![],
            repeats: 1,
            split_pos: vec![],
            seqovl_pattern: None,
            fooling: Fooling::default(),
            cutoff: None,
            any_protocol: false,
            fake_tls: None,
            fake_quic: None,
            fake_http: None,
            fake_discord: None,
            fake_unknown_udp: None,
            ip_id_zero: false,
        }
    }
}

/// Техника на уровне движка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleTechnique {
    Fake,
    Multisplit,
    Multidisorder,
    FakeSplit,
    HostFakeSplit,
}

/// Правило движка.
#[derive(Clone)]
pub struct EngineRule {
    pub name: String,
    pub protocol: L4Proto,
    /// Диапазоны портов назначения (включительно), уже с GameFilter.
    pub ports: Vec<(u16, u16)>,
    /// Hostlist-матчер (включающий, с запечёнными исключениями).
    pub host_match: Option<HostMatchFn>,
    /// Ipset-матчер (включающий, с исключениями).
    pub ip_match: Option<IpMatchFn>,
    /// L7-фильтр (--filter-l7): пусто = tls/quic/http (+unknown при
    /// any_protocol).
    pub l7: Vec<TrafficKind>,
    pub action: RuleAction,
}

impl std::fmt::Debug for EngineRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineRule")
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("ports", &self.ports)
            .field("l7", &self.l7)
            .field("host_match", &self.host_match.is_some())
            .field("ip_match", &self.ip_match.is_some())
            .finish()
    }
}

impl EngineRule {
    /// Совпадает ли пакет с фильтром правила.
    pub fn matches(&self, info: &PacketInfo, buf: &[u8], kind: TrafficKind) -> bool {
        // Протокол
        if info.proto != self.protocol {
            return false;
        }
        // Порты
        if !self.ports.is_empty()
            && !self
                .ports
                .iter()
                .any(|&(lo, hi)| lo <= info.dst_port && info.dst_port <= hi)
        {
            return false;
        }
        // L7
        if !self.l7.is_empty() {
            if !self.l7.contains(&kind) {
                return false;
            }
        } else {
            // Без явного l7: распознанные tls/quic/http; unknown —
            // только при any_protocol.
            let recognized = matches!(
                kind,
                TrafficKind::Tls | TrafficKind::Quic | TrafficKind::Http
            );
            if !recognized && !self.action.any_protocol {
                return false;
            }
            if kind == TrafficKind::Unknown && self.host_match.is_none() && self.ip_match.is_none()
            {
                // unknown без списков и без any_protocol — не применяем
                if !self.action.any_protocol {
                    return false;
                }
            }
        }
        // Hostlist (требует извлечённого хоста)
        if let Some(hm) = &self.host_match {
            let host = extract_host(info, buf);
            match host {
                Some(h) if hm(&h) => {}
                _ => return false,
            }
        }
        // Ipset
        if let Some(im) = &self.ip_match {
            match info.dst {
                super::packet::IpAddr::V4(ip) if im(ip) => {}
                _ => return false,
            }
        }
        true
    }
}

/// Классифицировать трафик пакета.
pub fn classify(info: &PacketInfo, buf: &[u8]) -> TrafficKind {
    if info.payload_len == 0 {
        return TrafficKind::Unknown;
    }
    let payload = &buf[info.payload_off..info.payload_off + info.payload_len];
    match info.proto {
        L4Proto::Tcp => {
            if info.is_tls_client_hello(buf) {
                TrafficKind::Tls
            } else if sni::extract_http_host(payload).is_some() {
                TrafficKind::Http
            } else {
                TrafficKind::Unknown
            }
        }
        L4Proto::Udp => {
            if info.is_quic_initial(buf) {
                TrafficKind::Quic
            } else if is_stun(payload) {
                TrafficKind::DiscordStun
            } else {
                TrafficKind::Unknown
            }
        }
        L4Proto::Other(_) => TrafficKind::Unknown,
    }
}

/// STUN: первые 2 бита 00, magic cookie 0x2112A442 (RFC 5389).
fn is_stun(payload: &[u8]) -> bool {
    payload.len() >= 20 && payload[0] & 0xC0 == 0 && payload[4..8] == [0x21, 0x12, 0xA4, 0x42]
}

/// Извлечь хост (SNI для TLS, Host для HTTP).
pub fn extract_host(info: &PacketInfo, buf: &[u8]) -> Option<String> {
    if info.payload_len == 0 {
        return None;
    }
    let payload = &buf[info.payload_off..info.payload_off + info.payload_len];
    sni::extract_tls_sni(payload)
        .map(|s| s.host)
        .or_else(|| sni::extract_http_host(payload))
}

// --- Таблица соединений для cutoff ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FlowKey {
    proto: u8,
    src: [u8; 16],
    dst: [u8; 16],
    sport: u16,
    dport: u16,
    is_v6: bool,
}

fn flow_key(info: &PacketInfo) -> FlowKey {
    let (src, dst, is_v6) = match (info.src, info.dst) {
        (super::packet::IpAddr::V4(s), super::packet::IpAddr::V4(d)) => {
            let mut sa = [0u8; 16];
            let mut da = [0u8; 16];
            sa[..4].copy_from_slice(&s.octets());
            da[..4].copy_from_slice(&d.octets());
            (sa, da, false)
        }
        (super::packet::IpAddr::V6(s), super::packet::IpAddr::V6(d)) => {
            (s.octets(), d.octets(), true)
        }
        _ => ([0; 16], [0; 16], false),
    };
    FlowKey {
        proto: match info.proto {
            L4Proto::Tcp => 6,
            L4Proto::Udp => 17,
            L4Proto::Other(p) => p,
        },
        src,
        dst,
        sport: info.src_port,
        dport: info.dst_port,
        is_v6,
    }
}

/// Учёт пакетов соединений для cutoff (nN).
/// Записи старше TTL удаляются при периодической чистке.
pub struct FlowTable {
    map: HashMap<FlowKey, (u32, Instant)>,
    last_gc: Instant,
}

const FLOW_TTL: Duration = Duration::from_secs(120);
const GC_INTERVAL: Duration = Duration::from_secs(30);

impl Default for FlowTable {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            last_gc: Instant::now(),
        }
    }
}

impl FlowTable {
    /// true — desync ещё разрешён для этого соединения
    /// (и счётчик увеличен). cutoff=None → всегда true.
    pub fn allow(&mut self, info: &PacketInfo, cutoff: Option<u32>) -> bool {
        let Some(limit) = cutoff else {
            return true;
        };
        if self.last_gc.elapsed() > GC_INTERVAL {
            self.map.retain(|_, (_, ts)| ts.elapsed() < FLOW_TTL);
            self.last_gc = Instant::now();
        }
        let entry = self
            .map
            .entry(flow_key(info))
            .or_insert((0, Instant::now()));
        entry.1 = Instant::now();
        if entry.0 >= limit {
            return false;
        }
        entry.0 += 1;
        true
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Разрешить вычисляемые split-позиции по пакету (sniext+N, midsld).
pub fn resolve_split_positions(rule: &EngineRule, buf: &[u8], info: &PacketInfo) -> Vec<usize> {
    let payload = &buf[info.payload_off..info.payload_off + info.payload_len];
    let sni = sni::extract_tls_sni(payload);
    rule.action
        .split_pos
        .iter()
        .filter_map(|p| match *p {
            RuleSplitPos::Absolute(n) => Some(n),
            RuleSplitPos::SniExtPlus(n) => {
                // начало расширения server_name = name_off - 9
                // (ext header 4 + list 2 + type 1 + name_len 2)
                sni.as_ref().map(|s| s.name_off.saturating_sub(9) + n)
            }
            RuleSplitPos::MidSld => sni.as_ref().and_then(|s| {
                // середина домена второго уровня: для a.b.c — «b»
                let host = &s.host;
                let parts: Vec<&str> = host.split('.').collect();
                if parts.len() < 2 {
                    return None;
                }
                let sld = parts[parts.len() - 2];
                let sld_start = host.rfind(sld).unwrap_or(0);
                Some(s.name_off + sld_start + sld.len() / 2)
            }),
        })
        .filter(|&pos| pos > 0 && pos < info.payload_len)
        .collect()
}

/// Применить действие правила к пакету → список исходящих пакетов.
pub fn apply(rule: &EngineRule, buf: &[u8], info: &PacketInfo, kind: TrafficKind) -> Vec<Outgoing> {
    let a = &rule.action;
    let original = || {
        desync::process(
            buf,
            info,
            desync::DesyncMode::None,
            &Fooling::default(),
            None,
        )
    };

    // Фейковый payload по классу трафика.
    let fake_payload: Option<&[u8]> = match kind {
        TrafficKind::Tls => a.fake_tls.as_deref(),
        TrafficKind::Quic => a.fake_quic.as_deref(),
        TrafficKind::Http => a.fake_http.as_deref(),
        TrafficKind::DiscordStun => a.fake_discord.as_deref(),
        TrafficKind::Unknown => {
            if info.proto == L4Proto::Udp {
                a.fake_unknown_udp.as_deref()
            } else {
                None
            }
        }
    };

    let mut out: Vec<Outgoing> = Vec::new();
    for tech in &a.techniques {
        match tech {
            RuleTechnique::Fake => {
                let mode = desync::DesyncMode::Fake;
                // повторяем фейк repeats раз (winws: repeats фейков)
                for _ in 0..a.repeats.max(1) {
                    // process(Fake) добавляет оригинал — берём только фейк
                    let v = desync::process(buf, info, mode, &a.fooling, fake_payload);
                    out.extend(v.into_iter().filter(|o| o.is_fake));
                }
            }
            RuleTechnique::Multisplit | RuleTechnique::Multidisorder => {
                let positions = resolve_split_positions(rule, buf, info);
                let segments = if let Some(pattern) = &a.seqovl_pattern {
                    // seqovl: одна позиция (первая валидная)
                    positions
                        .first()
                        .and_then(|&pos| desync::split_tcp_seqovl(buf, info, pos, pattern))
                        .map(|(p1, p2)| vec![p1, p2])
                        .unwrap_or_default()
                } else {
                    desync::split_tcp_multi(buf, info, &positions)
                };
                if segments.is_empty() {
                    return original();
                }
                if *tech == RuleTechnique::Multidisorder {
                    out.extend(segments.into_iter().rev());
                } else {
                    out.extend(segments);
                }
            }
            RuleTechnique::FakeSplit | RuleTechnique::HostFakeSplit => {
                for _ in 0..a.repeats.max(1) {
                    let v = desync::process(
                        buf,
                        info,
                        desync::DesyncMode::Fake,
                        &a.fooling,
                        fake_payload,
                    );
                    out.extend(v.into_iter().filter(|o| o.is_fake));
                }
                let positions = resolve_split_positions(rule, buf, info);
                let segments = desync::split_tcp_multi(buf, info, &positions);
                if segments.is_empty() {
                    out.extend(original());
                } else {
                    out.extend(segments);
                }
            }
        }
    }

    if out.is_empty() {
        return original();
    }
    // Действие должно содержать оригинал/его части: если техники дали
    // только фейки (Fake), добавляем оригинал в конец.
    let has_real = out.iter().any(|o| !o.is_fake);
    if !has_real {
        out.extend(original());
    }
    // ip-id=zero — для всех исходящих пакетов правила.
    if a.ip_id_zero {
        for o in &mut out {
            desync::zero_ip_id(&mut o.data, info);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp_info(payload: &[u8]) -> (Vec<u8>, PacketInfo) {
        let mut p = vec![0u8; 40 + payload.len()];
        p[0] = 0x45;
        let total = (p.len() as u16).to_be_bytes();
        p[2] = total[0];
        p[3] = total[1];
        p[9] = 6;
        p[16..20].copy_from_slice(&[203, 0, 113, 5]);
        p[20..22].copy_from_slice(&5000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[24..28].copy_from_slice(&1000u32.to_be_bytes());
        p[32] = 0x50;
        p[33] = 0x18;
        p[40..].copy_from_slice(payload);
        let info = PacketInfo::parse(&p).unwrap();
        (p, info)
    }

    fn base_rule() -> EngineRule {
        EngineRule {
            name: "t".into(),
            protocol: L4Proto::Tcp,
            ports: vec![(443, 443)],
            host_match: None,
            ip_match: None,
            l7: vec![],
            action: RuleAction {
                techniques: vec![RuleTechnique::Multisplit],
                split_pos: vec![RuleSplitPos::Absolute(2)],
                ..Default::default()
            },
        }
    }

    #[test]
    fn classify_tls_and_http() {
        let (p, i) = tcp_info(b"\x16\x03\x01\x00\x00");
        assert_eq!(classify(&i, &p), TrafficKind::Tls);
        let (p, i) = tcp_info(b"GET / HTTP/1.1\r\nHost: x.com\r\n\r\n");
        assert_eq!(classify(&i, &p), TrafficKind::Http);
        let (p, i) = tcp_info(b"\xde\xad\xbe\xef");
        assert_eq!(classify(&i, &p), TrafficKind::Unknown);
    }

    #[test]
    fn rule_ports_and_protocol() {
        let r = base_rule();
        let (p, i) = tcp_info(b"\x16\x03\x01xx");
        assert!(r.matches(&i, &p, TrafficKind::Tls));
        let mut i2 = i;
        i2.dst_port = 80;
        assert!(!r.matches(&i2, &p, TrafficKind::Tls));
        let mut i3 = i;
        i3.proto = L4Proto::Udp;
        assert!(!r.matches(&i3, &p, TrafficKind::Quic));
    }

    #[test]
    fn rule_hostlist_requires_host() {
        let mut r = base_rule();
        r.host_match = Some(Arc::new(|h: &str| h == "ok.com"));
        let (p, i) = tcp_info(b"\x16\x03\x01xx"); // нечитаемый ClientHello
        assert!(!r.matches(&i, &p, TrafficKind::Tls)); // нет SNI → нет матча
    }

    #[test]
    fn apply_multisplit_produces_parts() {
        let r = base_rule();
        let (p, i) = tcp_info(b"\x16\x03\x01abcdefgh");
        let out = apply(&r, &p, &i, TrafficKind::Tls);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|o| !o.is_fake));
    }

    #[test]
    fn apply_fake_repeats_and_original() {
        let mut r = base_rule();
        r.action.techniques = vec![RuleTechnique::Fake];
        r.action.repeats = 3;
        r.action.fake_tls = Some(vec![0x16; 50]);
        let (p, i) = tcp_info(b"\x16\x03\x01abcdefgh");
        let out = apply(&r, &p, &i, TrafficKind::Tls);
        assert_eq!(out.len(), 4); // 3 фейка + оригинал
        assert_eq!(out.iter().filter(|o| o.is_fake).count(), 3);
    }

    #[test]
    fn flow_cutoff_blocks_after_limit() {
        let (_p, i) = tcp_info(b"\x16\x03\x01abcdefgh");
        let mut ft = FlowTable::default();
        assert!(ft.allow(&i, Some(2)));
        assert!(ft.allow(&i, Some(2)));
        assert!(!ft.allow(&i, Some(2)));
        assert!(ft.allow(&i, None));
        assert_eq!(ft.len(), 1);
    }
}
