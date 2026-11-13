//! Парсер winws-аргументов оригинала (.bat) в типизированный Profile.
//!
//! Импорт — одноразовая миграция (Цикл 8), не рантайм-функция
//! (MODULE_CONTRACTS п.6). Поддерживаемые ключи — подмножество winws,
//! используемое 24 стратегиями v1.10.4 (STRATEGY_COMPATIBILITY п.1-2).
//! Неизвестные ключи не отбрасываются молча: собираются в список
//! unsupported для отчёта валидации.

use super::profile::{
    DesyncSpec, DesyncTechnique, FoolingKind, Profile, ProfileMeta, ProfileRule, Protocol, SplitPos,
};
use super::StrategyError;

/// Извлечь строку аргументов winws из содержимого .bat:
/// найти вызов winws.exe, склеить `^`-продолжения, убрать путь к exe,
/// развернуть %BIN% → bin/, %LISTS% → lists/.
pub fn extract_winws_args(bat: &str) -> Option<String> {
    let joined: String = bat
        .lines()
        .map(|l| {
            l.strip_suffix('^')
                .map(|s| s.to_string() + " ")
                .unwrap_or_else(|| l.to_string() + " ")
        })
        .collect();
    let idx = joined.find("winws.exe")?;
    let mut args = joined[idx + "winws.exe".len()..].to_string();
    // убрать всё после конца логической строки запуска (следующая команда bat)
    if let Some(end) = args.find("\r\n").or_else(|| args.find('\n')) {
        // в склеенной строке переводы строк остались; winws-вызов —
        // до первого перевода строки, НЕ завершённого ^
        let _ = end;
    }
    let args = {
        // вырезаем хвост после последнего известного ключа-аргумента:
        // ищем начало следующей bat-команды (echo, exit, pause и т.п.)
        let mut cut = args.len();
        for marker in ["\necho", "\nexit", "\npause", "\ncall ", "\nset ", "\ncd "] {
            if let Some(p) = args.to_lowercase().find(marker) {
                cut = cut.min(p);
            }
        }
        args.truncate(cut);
        args
    };
    let args = args
        .replace("%BIN%", "bin/")
        .replace("%LISTS%", "lists/")
        .replace("%~dp0", "")
        .replace(['%', '"'], "");
    Some(args)
}

/// Разобрать строку аргументов winws в Profile.
/// Возвращает (Profile, unsupported_keys).
pub fn parse_winws_args(name: &str, args: &str) -> Result<(Profile, Vec<String>), StrategyError> {
    let tokens = tokenize(args);
    let mut wf_tcp = String::new();
    let mut wf_udp = String::new();
    let mut rules: Vec<ProfileRule> = Vec::new();
    let mut cur: Option<ProfileRule> = None;
    let mut unsupported: Vec<String> = Vec::new();

    macro_rules! flush {
        () => {
            if let Some(r) = cur.take() {
                rules.push(r);
            }
        };
    }

    for tok in tokens {
        let (key, value) = match tok.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (tok.as_str(), None),
        };
        match key {
            "--new" => flush!(),
            "--wf-tcp" => wf_tcp = value.unwrap_or_default(),
            "--wf-udp" => wf_udp = value.unwrap_or_default(),
            "--filter-tcp" | "--filter-udp" => {
                flush!();
                cur = Some(ProfileRule {
                    protocol: if key == "--filter-tcp" {
                        Protocol::Tcp
                    } else {
                        Protocol::Udp
                    },
                    ports: value.unwrap_or_default(),
                    l7: vec![],
                    hostlists: vec![],
                    hostlist_domains: vec![],
                    hostlist_exclude: vec![],
                    hostlist_exclude_domains: vec![],
                    ipsets: vec![],
                    ipset_exclude: vec![],
                    desync: DesyncSpec::default(),
                    ip_id_zero: false,
                });
            }
            "--filter-l3" => {
                // ipv4 — единственное значение в стратегиях оригинала;
                // MXDS работает только на NETWORK-слое, фильтр L3
                // эквивалентен "все IPv4" — не ограничивает правило.
                if value.as_deref() != Some("ipv4") {
                    unsupported.push(format!("--filter-l3={}", value.unwrap_or_default()));
                }
            }
            "--filter-l7" => {
                if let Some(r) = cur.as_mut() {
                    r.l7 = value
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--hostlist" => push(&mut cur, |r, v| r.hostlists.push(v), &value),
            "--hostlist-exclude" => push(&mut cur, |r, v| r.hostlist_exclude.push(v), &value),
            "--hostlist-exclude-domains" => {
                if let Some(r) = cur.as_mut() {
                    r.hostlist_exclude_domains = value
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--hostlist-domains" => {
                if let Some(r) = cur.as_mut() {
                    r.hostlist_domains = value
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--ipset" => push(&mut cur, |r, v| r.ipsets.push(v), &value),
            "--ipset-exclude" => push(&mut cur, |r, v| r.ipset_exclude.push(v), &value),
            "--ip-id" => {
                if value.as_deref() == Some("zero") {
                    if let Some(r) = cur.as_mut() {
                        r.ip_id_zero = true;
                    }
                }
            }
            "--dpi-desync" => {
                if let Some(r) = cur.as_mut() {
                    r.desync.techniques = value
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|t| match t.trim() {
                            "fake" => Some(DesyncTechnique::Fake),
                            "multisplit" => Some(DesyncTechnique::Multisplit),
                            "multidisorder" => Some(DesyncTechnique::Multidisorder),
                            "syndata" => Some(DesyncTechnique::Syndata),
                            "hostfakesplit" => Some(DesyncTechnique::HostFakeSplit),
                            "fakedsplit" => Some(DesyncTechnique::FakeSplit),
                            other => {
                                unsupported.push(format!("--dpi-desync={other}"));
                                None
                            }
                        })
                        .collect();
                }
            }
            "--dpi-desync-repeats" => set(
                &mut cur,
                |r, v| r.desync.repeats = v.parse().unwrap_or(1),
                &value,
            ),
            "--dpi-desync-split-pos" => {
                if let Some(r) = cur.as_mut() {
                    r.desync.split_pos = value
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|p| {
                            let p = p.trim();
                            if let Some(rest) = p.strip_prefix("sniext+") {
                                rest.parse().ok().map(SplitPos::SniExtPlus)
                            } else if p == "midsld" {
                                Some(SplitPos::MidSld)
                            } else {
                                p.parse().ok().map(SplitPos::Absolute)
                            }
                        })
                        .collect();
                }
            }
            "--dpi-desync-split-seqovl" => {
                set(&mut cur, |r, v| r.desync.seqovl = v.parse().ok(), &value)
            }
            "--dpi-desync-split-seqovl-pattern" => {
                set(&mut cur, |r, v| r.desync.seqovl_pattern = Some(v), &value)
            }
            "--dpi-desync-fooling" => {
                if let Some(r) = cur.as_mut() {
                    r.desync.fooling = value
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|f| match f.trim() {
                            "ts" => Some(FoolingKind::Ts),
                            "badseq" => Some(FoolingKind::BadSeq),
                            "md5sig" => Some(FoolingKind::Md5Sig),
                            other => {
                                unsupported.push(format!("--dpi-desync-fooling={other}"));
                                None
                            }
                        })
                        .collect();
                }
            }
            "--dpi-desync-badseq-increment" => set(
                &mut cur,
                |r, v| r.desync.badseq_increment = v.parse().unwrap_or(-66000),
                &value,
            ),
            "--dpi-desync-cutoff" => {
                if let Some(r) = cur.as_mut() {
                    if let Some(v) = &value {
                        // формат nN
                        r.desync.cutoff = v.trim_start_matches('n').parse().ok();
                    }
                }
            }
            "--dpi-desync-any-protocol" => {
                set(&mut cur, |r, v| r.desync.any_protocol = v == "1", &value)
            }
            "--dpi-desync-fake-tls-mod" => {
                set(&mut cur, |r, v| r.desync.fake_tls_mod = Some(v), &value)
            }
            "--dpi-desync-hostfakesplit-mod" => set(
                &mut cur,
                |r, v| r.desync.hostfakesplit_mod = Some(v),
                &value,
            ),
            "--dpi-desync-fakedsplit-pattern" => set(
                &mut cur,
                |r, v| r.desync.fakedsplit_pattern = Some(v),
                &value,
            ),
            k if k.starts_with("--dpi-desync-fake-") => {
                let l7key = k.trim_start_matches("--dpi-desync-fake-");
                if let (Some(r), Some(v)) = (cur.as_mut(), value) {
                    r.desync.fakes.insert(l7key.to_string(), v);
                }
            }
            other if !other.trim().is_empty() && other.starts_with("--") => {
                unsupported.push(other.to_string());
            }
            _ => {}
        }
    }
    flush!();

    if rules.is_empty() {
        return Err(StrategyError::Parse {
            file: name.to_string(),
            detail: "нет ни одного правила (--filter-tcp/--filter-udp)".into(),
        });
    }

    let profile = Profile {
        id: slug(name),
        meta: ProfileMeta {
            name: name.to_string(),
            description: format!("Импортировано из стратегии оригинала «{name}»"),
            version: "1.0".into(),
            origin: Some("flowseal-1.10.4".into()),
        },
        wf_tcp,
        wf_udp,
        rules,
        compat: super::profile::CompatStatus::NotTested,
    };
    Ok((profile, unsupported))
}

fn push(cur: &mut Option<ProfileRule>, f: impl Fn(&mut ProfileRule, String), v: &Option<String>) {
    if let (Some(r), Some(v)) = (cur.as_mut(), v.clone()) {
        f(r, v);
    }
}

fn set(cur: &mut Option<ProfileRule>, f: impl Fn(&mut ProfileRule, String), v: &Option<String>) {
    if let (Some(r), Some(v)) = (cur.as_mut(), v.clone()) {
        f(r, v);
    }
}

/// Токенизация: пробелы как разделители (кавычки уже сняты
/// extract_winws_args; пути оригинала пробелов не содержат).
fn tokenize(args: &str) -> Vec<String> {
    args.split_whitespace()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Slug имени профиля: "general (ALT2)" -> "general-alt2".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERAL: &str = r#"start "zapret" /min "%BIN%winws.exe" --wf-tcp=80,443,%GameFilterTCP% --wf-udp=443,19294-19344,%GameFilterUDP% ^
--filter-udp=443 --hostlist="%LISTS%list-general.txt" --dpi-desync=fake --dpi-desync-repeats=6 --dpi-desync-fake-quic="%BIN%quic_initial_www_google_com.bin" --new ^
--filter-tcp=443 --hostlist="%LISTS%list-google.txt" --ip-id=zero --dpi-desync=multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-split-seqovl-pattern="%BIN%tls_clienthello_www_google_com.bin"
"#;

    #[test]
    fn extract_and_parse_general() {
        let args = extract_winws_args(GENERAL).unwrap();
        let (p, unsup) = parse_winws_args("general", &args).unwrap();
        assert_eq!(p.id, "general");
        assert!(unsup.is_empty(), "неподдержанные: {unsup:?}");
        assert_eq!(p.wf_tcp, "80,443,GameFilterTCP");
        assert_eq!(p.rules.len(), 2);

        let r0 = &p.rules[0];
        assert_eq!(r0.protocol, Protocol::Udp);
        assert_eq!(r0.ports, "443");
        assert_eq!(r0.hostlists, vec!["lists/list-general.txt"]);
        assert_eq!(r0.desync.techniques, vec![DesyncTechnique::Fake]);
        assert_eq!(r0.desync.repeats, 6);
        assert_eq!(
            r0.desync.fakes.get("quic").unwrap(),
            "bin/quic_initial_www_google_com.bin"
        );

        let r1 = &p.rules[1];
        assert_eq!(r1.protocol, Protocol::Tcp);
        assert!(r1.ip_id_zero);
        assert_eq!(r1.desync.techniques, vec![DesyncTechnique::Multisplit]);
        assert_eq!(r1.desync.seqovl, Some(681));
        assert_eq!(r1.desync.split_pos, vec![SplitPos::Absolute(1)]);
        assert_eq!(
            r1.desync.seqovl_pattern.as_deref(),
            Some("bin/tls_clienthello_www_google_com.bin")
        );
    }

    #[test]
    fn slug_cases() {
        assert_eq!(
            slug("general (FAKE TLS AUTO ALT2)"),
            "general-fake-tls-auto-alt2"
        );
        assert_eq!(slug("general"), "general");
        assert_eq!(slug("general (EXP)"), "general-exp");
    }

    #[test]
    fn unsupported_collected() {
        let args = "--wf-tcp=80 --filter-tcp=80 --dpi-desync=fake --dpi-desync-unknown-key=1 --new --filter-udp=443 --dpi-desync=fake";
        let (p, unsup) = parse_winws_args("t", args).unwrap();
        assert_eq!(p.rules.len(), 2);
        assert!(unsup.iter().any(|u| u.contains("unknown-key")));
    }

    #[test]
    fn cutoff_and_fooling() {
        let args = "--wf-tcp=80 --filter-tcp=80 --dpi-desync=fake,multisplit --dpi-desync-fooling=ts,badseq --dpi-desync-cutoff=n3 --dpi-desync-split-pos=1,midsld,sniext+1";
        let (p, _) = parse_winws_args("t", args).unwrap();
        let d = &p.rules[0].desync;
        assert_eq!(d.cutoff, Some(3));
        assert_eq!(d.fooling, vec![FoolingKind::Ts, FoolingKind::BadSeq]);
        assert_eq!(
            d.split_pos,
            vec![
                SplitPos::Absolute(1),
                SplitPos::MidSld,
                SplitPos::SniExtPlus(1)
            ]
        );
    }
}
