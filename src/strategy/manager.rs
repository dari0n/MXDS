//! Strategy Manager: обнаружение, валидация, резолвинг профилей.
//! Контракт: docs/MODULE_CONTRACTS.md п.6.
//!
//! Профили — JSON-файлы в profiles/ (типизированный формат MXDS,
//! импортированный из winws-стратегий оригинала парсером parser.rs).
//! Резолвинг: Profile + Config + lists/ + assets/payloads → EnginePlan.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::desync::Fooling;
use crate::engine::filter::parse_port_ranges;
use crate::engine::{
    EnginePlan, EngineRule, L4Proto, RuleAction, RuleSplitPos, RuleTechnique, TrafficKind,
};

use super::lists::{load_list_file, HostMatcher, IpMatcher};
use super::profile::{CompatStatus, ProfileMeta};
use super::profile::{DesyncTechnique, FoolingKind, Profile, ProfileRule, Protocol, SplitPos};
use super::profile_compat::CompatStore;
use super::validation::{IssueSeverity, ValidationIssue};
use super::StrategyError;

/// Проверка slug идентификатора профиля: [a-z0-9-], не пустой,
/// начинается с буквы/цифры.
fn valid_slug(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id.chars().next().is_some_and(|c| c != '-')
}

/// Менеджер профилей стратегий.
pub struct StrategyManager {
    /// Каталог профилей (profiles/).
    pub profiles_dir: PathBuf,
    /// Каталог списков (lists/).
    pub lists_dir: PathBuf,
    /// Каталог payload'ов (assets/payloads/).
    pub payloads_dir: PathBuf,
    /// Корень приложения (для data/profile-compat.json).
    root: PathBuf,
    /// Статусы совместимости из data/profile-compat.json.
    compat: std::sync::RwLock<CompatStore>,
}

impl StrategyManager {
    pub fn new(root: &Path) -> Self {
        Self {
            profiles_dir: root.join("profiles"),
            lists_dir: root.join("lists"),
            payloads_dir: root.join("assets").join("payloads"),
            root: root.to_path_buf(),
            compat: std::sync::RwLock::new(CompatStore::load(root)),
        }
    }

    /// Наложение сохранённого статуса совместимости на профиль.
    fn apply_compat(&self, mut p: Profile) -> Profile {
        if let Some(rec) = self
            .compat
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&p.id)
        {
            p.compat = rec.status;
        }
        p
    }

    /// Обнаружение профилей: все *.json в каталоге. Невалидные файлы
    /// пропускаются с warning (не роняют остальные).
    pub fn discover(&self) -> Result<Vec<Profile>, StrategyError> {
        let mut out = Vec::new();
        let rd = std::fs::read_dir(&self.profiles_dir).map_err(|e| StrategyError::Parse {
            file: self.profiles_dir.display().to_string(),
            detail: e.to_string(),
        })?;
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match self.load_file(&path) {
                Ok(p) => out.push(self.apply_compat(p)),
                Err(e) => tracing::warn!(?path, ?e, "профиль пропущен"),
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn load_file(&self, path: &Path) -> Result<Profile, StrategyError> {
        let text = std::fs::read_to_string(path).map_err(|e| StrategyError::Parse {
            file: path.display().to_string(),
            detail: e.to_string(),
        })?;
        serde_json::from_str(&text).map_err(|e| StrategyError::Parse {
            file: path.display().to_string(),
            detail: e.to_string(),
        })
    }

    /// Загрузить профиль по id.
    pub fn load(&self, id: &str) -> Result<Profile, StrategyError> {
        let path = self.profiles_dir.join(format!("{id}.json"));
        if !path.exists() {
            return Err(StrategyError::NotFound(id.to_string()));
        }
        self.load_file(&path).map(|p| self.apply_compat(p))
    }

    /// Сохранить результат проверки совместимости профиля
    /// (data/profile-compat.json; файл профиля не меняется).
    pub fn mark_compat(
        &self,
        id: &str,
        status: CompatStatus,
        note: String,
    ) -> Result<(), StrategyError> {
        // Профиль должен существовать.
        self.load(id)?;
        {
            let mut store = self.compat.write().unwrap_or_else(|e| e.into_inner());
            store.set(id, status, note);
            store.save(&self.root).map_err(|e| StrategyError::Parse {
                file: CompatStore::path(&self.root).display().to_string(),
                detail: e.to_string(),
            })?;
        }
        tracing::info!(profile = %id, ?status, "статус совместимости сохранён");
        Ok(())
    }

    /// Заметка о проверке профиля (для отображения).
    pub fn compat_note(&self, id: &str) -> Option<String> {
        self.compat
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map(|r| r.note.clone())
    }

    /// Создать профиль: копией существующего (`from`) или пустым
    /// шаблоном. Идентификатор — slug [a-z0-9-]; файл profiles/<id>.json
    /// не должен существовать.
    pub fn create(
        &self,
        id: &str,
        from: Option<&str>,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<Profile, StrategyError> {
        if !valid_slug(id) {
            return Err(StrategyError::Parse {
                file: id.to_string(),
                detail: "id должен быть slug: [a-z0-9-], начинаться с буквы/цифры".into(),
            });
        }
        let path = self.profiles_dir.join(format!("{id}.json"));
        if path.exists() {
            return Err(StrategyError::Parse {
                file: id.to_string(),
                detail: "профиль с таким id уже существует".into(),
            });
        }

        let mut p = match from {
            Some(base) => {
                let mut p = self.load(base)?;
                p.meta.origin = Some(format!("custom:copy-of:{base}"));
                p
            }
            None => Profile {
                id: String::new(),
                meta: ProfileMeta {
                    name: String::new(),
                    description: String::new(),
                    version: "1.0".into(),
                    origin: Some("custom".into()),
                },
                wf_tcp: "80,443".into(),
                wf_udp: "443".into(),
                rules: vec![ProfileRule {
                    protocol: Protocol::Tcp,
                    ports: "443".into(),
                    l7: vec![],
                    hostlists: vec!["lists/list-general.txt".into()],
                    hostlist_domains: vec![],
                    hostlist_exclude: vec![],
                    hostlist_exclude_domains: vec![],
                    ipsets: vec![],
                    ipset_exclude: vec![],
                    desync: Default::default(),
                    ip_id_zero: false,
                }],
                compat: CompatStatus::NotTested,
            },
        };
        p.id = id.to_string();
        p.meta.name = name.unwrap_or_else(|| id.to_string());
        p.meta.description = description.unwrap_or_default();
        p.compat = CompatStatus::NotTested;

        let issues = self.validate(&p);
        if issues.iter().any(|i| i.severity == IssueSeverity::Error) {
            for i in &issues {
                tracing::warn!(field = %i.field, "создание профиля: {}", i.message);
            }
            return Err(StrategyError::ValidationFailed);
        }

        let json = serde_json::to_string_pretty(&p).map_err(|e| StrategyError::Parse {
            file: path.display().to_string(),
            detail: e.to_string(),
        })?;
        crate::storage::repository::atomic_write(&path, json.as_bytes()).map_err(|e| {
            StrategyError::Parse {
                file: path.display().to_string(),
                detail: e.to_string(),
            }
        })?;
        tracing::info!(profile = %id, from = ?from, "профиль создан");
        Ok(p)
    }

    /// Валидация профиля: структура + существование payload'ов.
    /// Список проблем; пустой = валиден.
    pub fn validate(&self, p: &Profile) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        if p.rules.is_empty() {
            issues.push(ValidationIssue {
                field: "rules".into(),
                message: "профиль без правил".into(),
                severity: IssueSeverity::Error,
            });
        }
        for (i, r) in p.rules.iter().enumerate() {
            let field = format!("rules[{i}]");
            if r.ports.is_empty() {
                issues.push(ValidationIssue {
                    field: field.clone(),
                    message: "пустой список портов".into(),
                    severity: IssueSeverity::Error,
                });
            } else if !r.ports.contains("GameFilter")
                && parse_port_ranges(
                    &r.ports
                        .replace("GameFilterTCP,", "")
                        .replace("GameFilterUDP,", ""),
                )
                .is_empty()
            {
                issues.push(ValidationIssue {
                    field: field.clone(),
                    message: format!("невалидные порты: {}", r.ports),
                    severity: IssueSeverity::Error,
                });
            }
            if r.desync.techniques.is_empty() {
                issues.push(ValidationIssue {
                    field: field.clone(),
                    message: "нет техник desync".into(),
                    severity: IssueSeverity::Warning,
                });
            }
            // payload'ы: файлы должны существовать
            for (l7, v) in &r.desync.fakes {
                if let Some(file) = v.strip_prefix("bin/") {
                    let path = self.payloads_dir.join(file);
                    if !path.exists() {
                        issues.push(ValidationIssue {
                            field: field.clone(),
                            message: format!("payload {l7}: нет файла {file}"),
                            severity: IssueSeverity::Error,
                        });
                    }
                }
            }
            if let Some(pat) = &r.desync.seqovl_pattern {
                if let Some(file) = pat.strip_prefix("bin/") {
                    if !self.payloads_dir.join(file).exists() {
                        issues.push(ValidationIssue {
                            field: field.clone(),
                            message: format!("seqovl-паттерн: нет файла {file}"),
                            severity: IssueSeverity::Error,
                        });
                    }
                }
            }
            // списки должны существовать (user-списки опциональны)
            for l in r.hostlists.iter().chain(r.ipsets.iter()) {
                if let Some(file) = l.strip_prefix("lists/") {
                    if !file.contains("-user") && !self.lists_dir.join(file).exists() {
                        issues.push(ValidationIssue {
                            field: field.clone(),
                            message: format!("список: нет файла {file}"),
                            severity: IssueSeverity::Warning,
                        });
                    }
                }
            }
            // неподдерживаемое в движке
            if r.desync.techniques.contains(&DesyncTechnique::Syndata) {
                issues.push(ValidationIssue {
                    field: field.clone(),
                    message: "syndata не реализована в движке (будет пропущена)".into(),
                    severity: IssueSeverity::Warning,
                });
            }
            if r.desync.fake_tls_mod.is_some() {
                issues.push(ValidationIssue {
                    field: field.clone(),
                    message: "fake-tls-mod не реализован (фейк идёт как есть)".into(),
                    severity: IssueSeverity::Warning,
                });
            }
        }
        issues
    }

    /// Резолвинг: Profile + Config → EnginePlan (компиляция списков,
    /// чтение payload'ов, генерация фильтра).
    pub fn resolve(
        &self,
        p: &Profile,
        cfg: &crate::config::Config,
    ) -> Result<EnginePlan, StrategyError> {
        // Глобальный фильтр захвата: wf_tcp/wf_udp профиля.
        let wf_tcp = expand_game_filter(&p.wf_tcp, &cfg.game_filter, true);
        let wf_udp = expand_game_filter(&p.wf_udp, &cfg.game_filter, false);
        let filter_str = build_capture_filter(&wf_tcp, &wf_udp);

        let mut rules = Vec::new();
        for r in &p.rules {
            rules.push(self.resolve_rule(r, cfg)?);
        }
        Ok(EnginePlan {
            filter: filter_str,
            rules,
        })
    }

    fn resolve_rule(
        &self,
        r: &ProfileRule,
        cfg: &crate::config::Config,
    ) -> Result<EngineRule, StrategyError> {
        // Порты (с GameFilter-маркером).
        let ports_str = expand_game_filter(&r.ports, &cfg.game_filter, r.protocol == Protocol::Tcp);
        let ports = parse_port_ranges(&ports_str);

        // Hostlist: включающие + встроенные домены; исключения.
        let host_match = if !r.hostlists.is_empty() || !r.hostlist_domains.is_empty() {
            let mut m = HostMatcher::default();
            for f in &r.hostlists {
                m.add_hostlist(&load_list_file(&self.lists_dir.join(strip_lists(f))));
            }
            m.add_domains(&r.hostlist_domains);
            for f in &r.hostlist_exclude {
                m.add_exclude(&load_list_file(&self.lists_dir.join(strip_lists(f))));
            }
            // встроенные домены-исключения (как exclude-список)
            let joined = r.hostlist_exclude_domains.join(
                "
",
            );
            m.add_exclude(&joined);
            Some(Arc::new(move |h: &str| m.matches(h)) as crate::engine::rule::HostMatchFn)
        } else {
            None
        };

        // Ipset (учитываем ipset_mode конфигурации: loaded = применять).
        let ip_match = if !r.ipsets.is_empty() && cfg.ipset_mode == crate::config::IpsetMode::Loaded
        {
            let mut m = IpMatcher::default();
            for f in &r.ipsets {
                m.add_ipset(&load_list_file(&self.lists_dir.join(strip_lists(f))));
            }
            for f in &r.ipset_exclude {
                m.add_exclude(&load_list_file(&self.lists_dir.join(strip_lists(f))));
            }
            m.finalize();
            Some(Arc::new(move |ip| m.matches(ip)) as crate::engine::rule::IpMatchFn)
        } else {
            None
        };

        // L7-классы.
        let l7: Vec<TrafficKind> =
            r.l7.iter()
                .filter_map(|s| match s.as_str() {
                    "tls" => Some(TrafficKind::Tls),
                    "quic" => Some(TrafficKind::Quic),
                    "http" => Some(TrafficKind::Http),
                    "discord" | "stun" => Some(TrafficKind::DiscordStun),
                    _ => None,
                })
                .collect();

        // Техники: Syndata отбрасывается (предупреждение уже в validate).
        let techniques = r
            .desync
            .techniques
            .iter()
            .filter_map(|t| match t {
                DesyncTechnique::Fake => Some(RuleTechnique::Fake),
                DesyncTechnique::Multisplit => Some(RuleTechnique::Multisplit),
                DesyncTechnique::Multidisorder => Some(RuleTechnique::Multidisorder),
                DesyncTechnique::FakeSplit => Some(RuleTechnique::FakeSplit),
                DesyncTechnique::HostFakeSplit => Some(RuleTechnique::HostFakeSplit),
                DesyncTechnique::Syndata => None,
            })
            .collect();

        let fooling = Fooling {
            bad_seq: r.desync.fooling.contains(&FoolingKind::BadSeq),
            badseq_increment: r
                .desync
                .fooling
                .contains(&FoolingKind::BadSeq)
                .then_some(r.desync.badseq_increment),
            corrupt_ts: r.desync.fooling.contains(&FoolingKind::Ts),
            ..Default::default()
        };

        let load_payload = |key: &str| -> Option<Vec<u8>> {
            r.desync
                .fakes
                .get(key)
                .and_then(|v| self.load_payload_value(v))
        };

        Ok(EngineRule {
            name: format!("{}:{}:{}", p_protocol(r.protocol), r.ports, r.l7.join(",")),
            protocol: match r.protocol {
                Protocol::Tcp => L4Proto::Tcp,
                Protocol::Udp => L4Proto::Udp,
            },
            ports,
            host_match,
            ip_match,
            l7,
            action: RuleAction {
                techniques,
                repeats: r.desync.repeats,
                split_pos: r
                    .desync
                    .split_pos
                    .iter()
                    .map(|p| match p {
                        SplitPos::Absolute(n) => RuleSplitPos::Absolute(*n),
                        SplitPos::SniExtPlus(n) => RuleSplitPos::SniExtPlus(*n),
                        SplitPos::MidSld => RuleSplitPos::MidSld,
                    })
                    .collect(),
                seqovl_pattern: r
                    .desync
                    .seqovl_pattern
                    .as_deref()
                    .and_then(|v| self.load_payload_value(v)),
                fooling,
                cutoff: r.desync.cutoff,
                any_protocol: r.desync.any_protocol,
                fake_tls: load_payload("tls"),
                fake_quic: load_payload("quic"),
                fake_http: load_payload("http"),
                fake_discord: load_payload("discord").or_else(|| load_payload("stun")),
                fake_unknown_udp: load_payload("unknown-udp").or_else(|| load_payload("unknown")),
                ip_id_zero: r.ip_id_zero,
            },
        })
    }

    /// Значение payload: "bin/file.bin" | "0xHEX" | "!" (один байт 0x00).
    fn load_payload_value(&self, v: &str) -> Option<Vec<u8>> {
        if let Some(file) = v.strip_prefix("bin/") {
            let path = self.payloads_dir.join(file);
            match std::fs::read(&path) {
                Ok(data) => Some(data),
                Err(_) => {
                    tracing::warn!(?path, "payload не найден");
                    None
                }
            }
        } else if let Some(hex) = v.strip_prefix("0x") {
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .ok()
        } else if v == "!" {
            Some(vec![0x00])
        } else {
            None
        }
    }
}

fn p_protocol(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

fn strip_lists(f: &str) -> &str {
    f.strip_prefix("lists/").unwrap_or(f)
}

/// Развернуть маркер GameFilterTCP/GameFilterUDP в строке портов.
fn expand_game_filter(ports: &str, gf: &crate::config::GameFilter, is_tcp: bool) -> String {
    let marker = if is_tcp {
        "GameFilterTCP"
    } else {
        "GameFilterUDP"
    };
    let replacement = match (&gf.mode, is_tcp) {
        (crate::config::GameFilterMode::Disabled, _) => "",
        (crate::config::GameFilterMode::Tcp, false) => "",
        (crate::config::GameFilterMode::Udp, true) => "",
        (_, true) => &gf.tcp_ports,
        (_, false) => &gf.udp_ports,
    };
    let expanded = ports.replace(marker, replacement);
    // убрать двойные запятые/висячие
    expanded
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

/// Строка фильтра WinDivert по развёрнутым wf-портам.
fn build_capture_filter(wf_tcp: &str, wf_udp: &str) -> String {
    let tcp: Vec<u16> = parse_port_ranges(wf_tcp)
        .iter()
        .map(|&(lo, _)| lo)
        .collect();
    // Для фильтра нужны и диапазоны: используем build_filter только для
    // одиночных портов; диапазоны собираем вручную.
    let _ = tcp;
    let mut conds = vec![];
    let tcp_ranges = parse_port_ranges(wf_tcp);
    if !tcp_ranges.is_empty() {
        let parts: Vec<String> = tcp_ranges
            .iter()
            .map(|&(lo, hi)| {
                if lo == hi {
                    format!("tcp.DstPort=={lo}")
                } else {
                    format!("(tcp.DstPort>={lo} and tcp.DstPort<={hi})")
                }
            })
            .collect();
        conds.push(format!("(tcp and ({}))", parts.join(" or ")));
    }
    let udp_ranges = parse_port_ranges(wf_udp);
    if !udp_ranges.is_empty() {
        let parts: Vec<String> = udp_ranges
            .iter()
            .map(|&(lo, hi)| {
                if lo == hi {
                    format!("udp.DstPort=={lo}")
                } else {
                    format!("(udp.DstPort>={lo} and udp.DstPort<={hi})")
                }
            })
            .collect();
        conds.push(format!("(udp and ({}))", parts.join(" or ")));
    }
    if conds.is_empty() {
        return "outbound and !loopback and tcp.DstPort==0 and udp.DstPort==0".into();
    }
    format!("outbound and !loopback and ({})", conds.join(" or "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::parser::parse_winws_args;
    use crate::strategy::profile::CompatStatus;

    fn mgr() -> StrategyManager {
        StrategyManager::new(Path::new(env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn resolve_general_profile() {
        let args = "--wf-tcp=80,443,GameFilterTCP --wf-udp=443 \
                    --filter-udp=443 --hostlist=lists/list-general.txt \
                    --dpi-desync=fake --dpi-desync-repeats=6 \
                    --dpi-desync-fake-quic=bin/quic_initial_www_google_com.bin --new \
                    --filter-tcp=443 --hostlist=lists/list-google.txt --ip-id=zero \
                    --dpi-desync=multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 \
                    --dpi-desync-split-seqovl-pattern=bin/tls_clienthello_www_google_com.bin";
        let (p, unsup) = parse_winws_args("general", args).unwrap();
        assert!(unsup.is_empty());
        let m = mgr();
        let issues = m.validate(&p);
        assert!(
            issues.iter().all(|i| i.severity != IssueSeverity::Error),
            "ошибки валидации: {issues:?}"
        );
        let cfg = crate::config::Config::default();
        let plan = m.resolve(&p, &cfg).unwrap();
        assert!(plan.filter.contains("tcp.DstPort==80"));
        assert!(plan.filter.contains("udp.DstPort==443"));
        assert_eq!(plan.rules.len(), 2);

        // Правило QUIC: fake с payload из файла.
        let quic = &plan.rules[0];
        assert_eq!(quic.protocol, L4Proto::Udp);
        assert!(quic.action.fake_quic.is_some());
        assert_eq!(quic.action.repeats, 6);

        // Правило Google: multisplit + seqovl + ip-id zero.
        let google = &plan.rules[1];
        assert_eq!(google.protocol, L4Proto::Tcp);
        assert!(google.action.ip_id_zero);
        assert!(google.action.seqovl_pattern.is_some());
        assert!(google.host_match.is_some());
    }

    #[test]
    fn game_filter_expansion() {
        let mut cfg = crate::config::Config::default();
        cfg.game_filter.mode = crate::config::GameFilterMode::All;
        cfg.game_filter.tcp_ports = "27015-27050".into();
        cfg.game_filter.udp_ports = "5000-6000".into();
        let tcp = expand_game_filter("80,443,GameFilterTCP", &cfg.game_filter, true);
        assert_eq!(tcp, "80,443,27015-27050");
        let udp = expand_game_filter("443,GameFilterUDP", &cfg.game_filter, false);
        assert_eq!(udp, "443,5000-6000");
        // Disabled: маркер исчезает.
        let cfg2 = crate::config::Config::default();
        assert_eq!(
            expand_game_filter("80,443,GameFilterTCP", &cfg2.game_filter, true),
            "80,443"
        );
    }

    #[test]
    fn validate_missing_payload() {
        let args = "--wf-tcp=443 --wf-udp=443 --filter-tcp=443 \
                    --dpi-desync=fake --dpi-desync-fake-tls=bin/nonexistent.bin";
        let (p, _) = parse_winws_args("t", args).unwrap();
        let issues = mgr().validate(&p);
        assert!(issues
            .iter()
            .any(|i| i.message.contains("nonexistent.bin") && i.severity == IssueSeverity::Error));
    }

    #[test]
    fn payload_hex_and_bang() {
        let m = mgr();
        assert_eq!(m.load_payload_value("0x0000FF"), Some(vec![0, 0, 255]));
        assert_eq!(m.load_payload_value("!"), Some(vec![0]));
        assert!(m.load_payload_value("0xZZ").is_none());
    }

    #[test]
    fn compat_status_default_not_tested() {
        let (p, _) =
            parse_winws_args("t", "--wf-tcp=80 --filter-tcp=80 --dpi-desync=fake").unwrap();
        assert_eq!(p.compat, CompatStatus::NotTested);
    }
}
