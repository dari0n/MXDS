//! Генератор строки фильтра WinDivert из типизированной конфигурации
//! (WINDIVERT_INTEGRATION п.4: фильтр не строится из сырого ввода).
//!
//! Формат языка фильтров WinDivert 2.2 (NETWORK-слой):
//! `outbound and !loopback and (tcp.DstPort==80 or udp.DstPort==443 ...)`.
//! Диапазоны Game Filter: `(tcp.DstPort>=A and tcp.DstPort<=B)`.

use crate::config::{GameFilter, GameFilterMode};

/// Разобрать строку портов "80,443,5000-6000" в диапазоны.
/// Вход предварительно проходит validate_port_ranges (Цикл 5);
/// невалидные элементы здесь молча пропускаются — дублирующая
/// ошибка не нужна, конфиг валидируется до запуска движка.
pub fn parse_port_ranges(input: &str) -> Vec<(u16, u16)> {
    input
        .replace(' ', "")
        .split(',')
        .filter_map(|item| {
            let (a, b) = match item.split_once('-') {
                Some((a, b)) => (a, b),
                None => (item, item),
            };
            let s = a.parse::<u16>().ok()?;
            let e = b.parse::<u16>().ok()?;
            (s >= 1 && s <= e).then_some((s, e))
        })
        .collect()
}

/// Построить фильтр из портов TCP, UDP и Game Filter.
/// Пустой результат (нет портов) — фильтр, не совпадающий ни с чем
/// полезным (нулевой порт), чтобы движок не захватывал весь трафик.
pub fn build_filter(tcp_ports: &[u16], udp_ports: &[u16], game: &GameFilter) -> String {
    let mut conds: Vec<String> = Vec::new();

    if !tcp_ports.is_empty() {
        let ports = tcp_ports
            .iter()
            .map(|p| format!("tcp.DstPort=={p}"))
            .collect::<Vec<_>>()
            .join(" or ");
        conds.push(format!("(tcp and ({}))", ports));
    }
    if !udp_ports.is_empty() {
        let ports = udp_ports
            .iter()
            .map(|p| format!("udp.DstPort=={p}"))
            .collect::<Vec<_>>()
            .join(" or ");
        conds.push(format!("(udp and ({}))", ports));
    }

    // Game Filter: режим определяет, какие протоколы добавлять.
    let (add_tcp, add_udp) = match game.mode {
        GameFilterMode::Disabled => (false, false),
        GameFilterMode::All => (true, true),
        GameFilterMode::Tcp => (true, false),
        GameFilterMode::Udp => (false, true),
    };
    let mut sub = Vec::new();
    if add_tcp {
        for (lo, hi) in parse_port_ranges(&game.tcp_ports) {
            sub.push(format!("(tcp.DstPort>={lo} and tcp.DstPort<={hi})"));
        }
    }
    if add_udp {
        for (lo, hi) in parse_port_ranges(&game.udp_ports) {
            sub.push(format!("(udp.DstPort>={lo} and udp.DstPort<={hi})"));
        }
    }
    if !sub.is_empty() {
        conds.push(format!("({})", sub.join(" or ")));
    }

    if conds.is_empty() {
        return "outbound and !loopback and tcp.DstPort==0 and udp.DstPort==0".into();
    }
    format!("outbound and !loopback and ({})", conds.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_game() -> GameFilter {
        GameFilter::default()
    }

    #[test]
    fn parse_ranges() {
        assert_eq!(
            parse_port_ranges("80,443,5000-6000"),
            vec![(80, 80), (443, 443), (5000, 6000)]
        );
        assert_eq!(parse_port_ranges("1024-65535"), vec![(1024, 65535)]);
        assert!(parse_port_ranges("").is_empty());
        assert!(parse_port_ranges("abc,100-50").is_empty());
        assert_eq!(
            parse_port_ranges("443, 8080-8081"),
            vec![(443, 443), (8080, 8081)]
        );
    }

    #[test]
    fn tcp_and_udp_ports() {
        let f = build_filter(&[80, 443], &[443], &no_game());
        assert_eq!(
            f,
            "outbound and !loopback and ((tcp and (tcp.DstPort==80 or tcp.DstPort==443)) or (udp and (udp.DstPort==443)))"
        );
    }

    #[test]
    fn game_filter_all_mode() {
        let g = GameFilter {
            mode: GameFilterMode::All,
            tcp_ports: "27015-27050".into(),
            udp_ports: "5000-6000".into(),
        };
        let f = build_filter(&[443], &[], &g);
        assert!(f.contains("tcp.DstPort==443"));
        assert!(f.contains("tcp.DstPort>=27015 and tcp.DstPort<=27050"));
        assert!(f.contains("udp.DstPort>=5000 and udp.DstPort<=6000"));
    }

    #[test]
    fn game_filter_tcp_only() {
        let g = GameFilter {
            mode: GameFilterMode::Tcp,
            tcp_ports: "1024-65535".into(),
            udp_ports: "5000-6000".into(),
        };
        let f = build_filter(&[], &[], &g);
        assert!(f.contains("tcp.DstPort>=1024"));
        assert!(!f.contains("udp.DstPort"));
    }

    #[test]
    fn empty_is_impossible_filter() {
        let f = build_filter(&[], &[], &no_game());
        assert!(f.contains("tcp.DstPort==0"));
    }
}
