//! Списки доменов и IP: загрузка и матчинг (lists/, D-004).
//!
//! Форматы оригинала:
//! - hostlist: домен на строку; `^domain` — точное совпадение,
//!   без `^` — домен и все поддомены (семантика zapret).
//! - ipset: IPv4 или CIDR на строку.

use std::net::Ipv4Addr;
use std::path::Path;

/// Матчер доменов: точные + суффиксные множества + исключения.
#[derive(Debug, Clone, Default)]
pub struct HostMatcher {
    exact: Vec<String>,
    suffix: Vec<String>,
    exact_exclude: Vec<String>,
    suffix_exclude: Vec<String>,
}

impl HostMatcher {
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.suffix.is_empty()
    }

    /// Добавить записи hostlist-файла (построчно).
    pub fn add_hostlist(&mut self, content: &str) {
        for line in content.lines() {
            let d = normalize_domain(line);
            if d.is_empty() {
                continue;
            }
            if let Some(stripped) = d.strip_prefix('^') {
                self.exact.push(stripped.to_string());
            } else {
                self.suffix.push(d);
            }
        }
    }

    /// Добавить домены --hostlist-domains=a,b (суффиксные, как в zapret).
    pub fn add_domains(&mut self, domains: &[String]) {
        for d in domains {
            let d = d.trim().to_lowercase();
            if !d.is_empty() {
                self.suffix.push(d);
            }
        }
    }

    /// Добавить исключения (hostlist-exclude).
    pub fn add_exclude(&mut self, content: &str) {
        for line in content.lines() {
            let d = normalize_domain(line);
            if d.is_empty() {
                continue;
            }
            if let Some(stripped) = d.strip_prefix('^') {
                self.exact_exclude.push(stripped.to_string());
            } else {
                self.suffix_exclude.push(d);
            }
        }
    }

    /// Совпадает ли хост (без учёта регистра; точка на конце допустима).
    pub fn matches(&self, host: &str) -> bool {
        let h = host.trim_end_matches('.').to_lowercase();
        if h.is_empty() {
            return false;
        }
        // Исключения приоритетнее (zapret: exclude проверяется первым).
        if self.exact_exclude.iter().any(|e| e == &h)
            || self.suffix_exclude.iter().any(|e| suffix_match(&h, e))
        {
            return false;
        }
        self.exact.iter().any(|e| e == &h) || self.suffix.iter().any(|s| suffix_match(&h, s))
    }
}

fn normalize_domain(line: &str) -> String {
    line.trim().trim_end_matches('.').to_lowercase()
}

fn suffix_match(host: &str, suffix: &str) -> bool {
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

/// Матчер IP: отсортированные интервалы IPv4 (u32), бинарный поиск.
/// IPv6-записей в списках оригинала нет; при появлении — игнорируются.
#[derive(Debug, Clone, Default)]
pub struct IpMatcher {
    ranges: Vec<(u32, u32)>, // включительно, отсортированы, слиты
    excludes: Vec<(u32, u32)>,
}

impl IpMatcher {
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn add_ipset(&mut self, content: &str) {
        for line in content.lines() {
            if let Some(r) = parse_ip_line(line) {
                self.ranges.push(r);
            }
        }
    }

    pub fn add_exclude(&mut self, content: &str) {
        for line in content.lines() {
            if let Some(r) = parse_ipset_line(line) {
                self.excludes.push(r);
            }
        }
    }

    /// Нормализация после всех добавлений: сортировка + слияние.
    pub fn finalize(&mut self) {
        Self::normalize(&mut self.ranges);
        Self::normalize(&mut self.excludes);
    }

    fn normalize(v: &mut Vec<(u32, u32)>) {
        v.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
        for &(lo, hi) in v.iter() {
            if let Some(last) = out.last_mut() {
                if lo <= last.1.saturating_add(1) {
                    last.1 = last.1.max(hi);
                    continue;
                }
            }
            out.push((lo, hi));
        }
        *v = out;
    }

    pub fn matches(&self, ip: Ipv4Addr) -> bool {
        let v = u32::from(ip);
        if Self::contains(&self.excludes, v) {
            return false;
        }
        Self::contains(&self.ranges, v)
    }

    fn contains(ranges: &[(u32, u32)], v: u32) -> bool {
        ranges
            .binary_search_by(|&(lo, hi)| {
                if v < lo {
                    std::cmp::Ordering::Greater
                } else if v > hi {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// Строка ipset для включения: "1.2.3.4" или "1.2.3.0/24".
/// Комментарии (# ...) и пустые строки игнорируются.
pub fn parse_ip_line(line: &str) -> Option<(u32, u32)> {
    parse_ipset_line(line)
}

fn parse_ipset_line(line: &str) -> Option<(u32, u32)> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() {
        return None;
    }
    match line.split_once('/') {
        Some((ip, mask)) => {
            let ip: Ipv4Addr = ip.trim().parse().ok()?;
            let bits: u32 = mask.trim().parse().ok()?;
            if bits > 32 {
                return None;
            }
            let host_mask = match bits {
                0 => u32::MAX,
                32 => 0,
                b => u32::MAX >> b,
            };
            let base = u32::from(ip) & !host_mask;
            Some((base, base | host_mask))
        }
        None => {
            let ip: Ipv4Addr = line.parse().ok()?;
            let v = u32::from(ip);
            Some((v, v))
        }
    }
}

/// Загрузить файл списка; отсутствие файла — пустой контент с warning
/// (user-списки могут не существовать — это норма).
pub fn load_list_file(path: &Path) -> String {
    match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            tracing::debug!(?path, "список не найден, пропускаем");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostlist_suffix_and_exact() {
        let mut m = HostMatcher::default();
        m.add_hostlist("^exact.example.com\nexample.org\n");
        assert!(m.matches("exact.example.com"));
        assert!(!m.matches("www.exact.example.com")); // ^ — только точное
        assert!(m.matches("example.org"));
        assert!(m.matches("cdn.example.org"));
        assert!(!m.matches("notexample.org"));
        assert!(!m.matches("example.org.evil.com"));
    }

    #[test]
    fn hostlist_exclude_wins() {
        let mut m = HostMatcher::default();
        m.add_hostlist("example.org\n");
        m.add_exclude("cdn.example.org\n");
        assert!(m.matches("example.org"));
        assert!(!m.matches("cdn.example.org"));
    }

    #[test]
    fn hostlist_case_and_trailing_dot() {
        let mut m = HostMatcher::default();
        m.add_hostlist("Example.ORG\n");
        assert!(m.matches("EXAMPLE.org."));
        assert!(m.matches("WWW.Example.Org"));
    }

    #[test]
    fn ipset_single_and_cidr() {
        let mut m = IpMatcher::default();
        m.add_ipset("10.0.0.0/8\n192.168.1.5\n# comment\n\n");
        m.finalize();
        assert!(m.matches("10.20.30.40".parse().unwrap()));
        assert!(m.matches("192.168.1.5".parse().unwrap()));
        assert!(!m.matches("192.168.1.6".parse().unwrap()));
        assert!(!m.matches("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn ipset_exclude_wins_and_merge() {
        let mut m = IpMatcher::default();
        m.add_ipset("10.0.0.0/24\n10.0.1.0/24\n");
        m.add_exclude("10.0.0.128/25\n");
        m.finalize();
        assert!(m.matches("10.0.1.1".parse().unwrap()));
        assert!(m.matches("10.0.0.10".parse().unwrap()));
        assert!(!m.matches("10.0.0.200".parse().unwrap()));
        // слияние соседних /24
        assert_eq!(m.ranges.len(), 1);
    }

    #[test]
    fn ipset_garbage_ignored() {
        assert!(parse_ip_line("").is_none());
        assert!(parse_ip_line("abc").is_none());
        assert!(parse_ip_line("10.0.0.0/33").is_none());
        assert!(parse_ip_line("2001:db8::/32").is_none()); // IPv6 не поддерживается
    }
}
