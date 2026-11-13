//! Конкретные проверки (Цикл 10). Состав — перенос Run Diagnostics
//! оригинала (FUNCTIONALITY_MAP F-11, service.bat): права, ОС, BFE,
//! файлы/драйвер WinDivert, TCP timestamps, прокси, DoH, конфликтующие
//! службы, стороннее ПО (Adguard/Killer/Intel Connectivity/Check Point/
//! SmartByte), VPN, кириллица/OneDrive в пути, hosts, целостность
//! конфигурации, состояние движка.
//!
//! Принципы:
//! - проверки read-only: ничего не меняют в системе;
//! - сбой одной проверки не роняет прогон (run_all изолирует паники);
//! - duration замеряется для каждой проверки.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::diagnostics::{Check, CheckResult, CheckStatus, DiagContext};
use crate::engine::EngineState;
use crate::strategy::IssueSeverity;
use crate::windivert::ffi;
use crate::windows::{components, integration, permissions};

/// Полный набор проверок по умолчанию (MODULE_CONTRACTS п.8).
pub fn default_checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(ElevationCheck),
        Box::new(OsVersionCheck),
        Box::new(BfeCheck),
        Box::new(WindivertFilesCheck),
        Box::new(WindivertDriverCheck),
        Box::new(TcpTimestampsCheck),
        Box::new(SystemProxyCheck),
        Box::new(SecureDnsCheck),
        Box::new(ConflictingServicesCheck),
        Box::new(ServicePatternCheck::ADGUARD),
        Box::new(ServicePatternCheck::KILLER),
        Box::new(ServicePatternCheck::INTEL_CONNECTIVITY),
        Box::new(ServicePatternCheck::CHECK_POINT),
        Box::new(ServicePatternCheck::SMARTBYTE),
        Box::new(ServicePatternCheck::VPN),
        Box::new(CyrillicPathCheck),
        Box::new(OneDrivePathCheck),
        Box::new(HostsFileCheck),
        Box::new(ConfigIntegrityCheck),
        Box::new(EngineStateCheck),
    ]
}

/// Прогон набора проверок. Паника отдельной проверки изолируется
/// в CheckResult со статусом Fail — остальные выполняются.
pub fn run_all(ctx: &DiagContext, checks: Vec<Box<dyn Check>>) -> Vec<CheckResult> {
    checks
        .iter()
        .map(|c| {
            let started = Instant::now();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.run(ctx))) {
                Ok(mut r) => {
                    r.duration = started.elapsed();
                    r
                }
                Err(_) => CheckResult {
                    name: c.name().to_string(),
                    status: CheckStatus::Fail,
                    description: "внутренняя ошибка проверки".into(),
                    duration: started.elapsed(),
                    details: Some("проверка завершилась паникой; см. лог".into()),
                    recommendation: Some("сообщите разработчикам MXDS".into()),
                    fix: None,
                },
            }
        })
        .collect()
}

fn result(
    name: &str,
    status: CheckStatus,
    description: impl Into<String>,
    details: Option<String>,
    recommendation: Option<String>,
) -> CheckResult {
    CheckResult {
        name: name.to_string(),
        status,
        description: description.into(),
        duration: Duration::ZERO, // перезаписывается в run_all
        details,
        recommendation,
        fix: None,
    }
}

/// Результат проверки с доступным авто-исправлением (FixId).
fn result_fix(
    name: &str,
    status: CheckStatus,
    description: impl Into<String>,
    details: Option<String>,
    recommendation: Option<String>,
    fix: super::FixId,
) -> CheckResult {
    let mut r = result(name, status, description, details, recommendation);
    r.fix = Some(fix);
    r
}

// --- Права ---

struct ElevationCheck;

impl Check for ElevationCheck {
    fn name(&self) -> &str {
        "elevation"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        if permissions::is_elevated() {
            result(
                self.name(),
                CheckStatus::Pass,
                "права администратора есть",
                None,
                None,
            )
        } else {
            result(
                self.name(),
                CheckStatus::Warn,
                "процесс без прав администратора",
                None,
                Some("запустите MXDS от имени администратора — без этого движок и часть проверок недоступны".into()),
            )
        }
    }
}

// --- Версия ОС ---

struct OsVersionCheck;

impl Check for OsVersionCheck {
    fn name(&self) -> &str {
        "os_version"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        match integration::os_version() {
            Ok(v) => {
                let desc = format!("Windows {}.{} build {}", v.major, v.minor, v.build);
                if integration::is_supported_os().unwrap_or(false) {
                    result(
                        self.name(),
                        CheckStatus::Pass,
                        desc,
                        Some("поддерживается (build >= 17763)".into()),
                        None,
                    )
                } else {
                    result(
                        self.name(),
                        CheckStatus::Fail,
                        desc,
                        Some("требуется Windows 10 1809+ (build 17763)".into()),
                        Some("обновите Windows".into()),
                    )
                }
            }
            Err(e) => result(
                self.name(),
                CheckStatus::Warn,
                "не удалось определить версию ОС",
                Some(e.to_string()),
                None,
            ),
        }
    }
}

// --- BFE ---

struct BfeCheck;

impl Check for BfeCheck {
    fn name(&self) -> &str {
        "bfe_service"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        match integration::bfe_running() {
            Ok(true) => result(
                self.name(),
                CheckStatus::Pass,
                "служба BFE (Base Filtering Engine) работает",
                None,
                None,
            ),
            Ok(false) => result(
                self.name(),
                CheckStatus::Fail,
                "служба BFE не работает",
                Some("WinDivert не сможет открыть драйвер без BFE".into()),
                Some("включите и запустите службу BFE в services.msc".into()),
            ),
            Err(e) => result(
                self.name(),
                CheckStatus::Warn,
                "не удалось проверить состояние BFE",
                Some(e.to_string()),
                None,
            ),
        }
    }
}

// --- WinDivert: файлы ---

struct WindivertFilesCheck;

impl WindivertFilesCheck {
    fn sys_path(dll_path: &Path) -> PathBuf {
        dll_path.with_file_name("WinDivert64.sys")
    }
}

impl Check for WindivertFilesCheck {
    fn name(&self) -> &str {
        "windivert_files"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        let dll = &ctx.dll_path;
        let sys = Self::sys_path(dll);
        if !dll.exists() {
            return result(
                self.name(),
                CheckStatus::Fail,
                "WinDivert.dll не найден",
                Some(format!("ожидался: {}", dll.display())),
                Some("переустановите MXDS".into()),
            );
        }
        if !sys.exists() {
            return result(
                self.name(),
                CheckStatus::Fail,
                "WinDivert64.sys не найден",
                Some(format!("ожидался: {}", sys.display())),
                Some("переустановите MXDS".into()),
            );
        }
        // Контроль целостности по эталонным SHA256 (SECURITY_REVIEW п.4).
        let dll_ok = ffi::sha256_file(dll)
            .map(|h| h.eq_ignore_ascii_case(ffi::EXPECTED_DLL_SHA256))
            .unwrap_or(false);
        let sys_ok = ffi::sha256_file(&sys)
            .map(|h| h.eq_ignore_ascii_case(ffi::EXPECTED_SYS_SHA256))
            .unwrap_or(false);
        if dll_ok && sys_ok {
            result(
                self.name(),
                CheckStatus::Pass,
                "бинарники WinDivert на месте, хэши совпадают",
                Some(format!("dll: {}", dll.display())),
                None,
            )
        } else {
            result(
                self.name(),
                CheckStatus::Fail,
                "хэш бинарников WinDivert не совпадает с эталоном",
                Some(format!("dll sha256 ok: {dll_ok}, sys sha256 ok: {sys_ok}")),
                Some("файлы подменены или повреждены — переустановите MXDS".into()),
            )
        }
    }
}

// --- WinDivert: драйвер ---

struct WindivertDriverCheck;

impl Check for WindivertDriverCheck {
    fn name(&self) -> &str {
        "windivert_driver"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        use components::ServiceStatus;
        let mut active = Vec::new();
        for name in ["WinDivert", "WinDivert14"] {
            match components::service_status(name) {
                Ok(ServiceStatus::NotFound) => {}
                Ok(ServiceStatus::Running) => active.push(format!("{name} (running)")),
                Ok(s) => active.push(format!("{name} ({s:?})")),
                Err(e) => {
                    return result(
                        self.name(),
                        CheckStatus::Warn,
                        "не удалось запросить состояние драйвера",
                        Some(e.to_string()),
                        None,
                    )
                }
            }
        }
        if active.is_empty() {
            return result(
                self.name(),
                CheckStatus::Pass,
                "драйвер WinDivert не загружен (ожидаемо вне работы движка)",
                None,
                None,
            );
        }
        let details = Some(active.join(", "));
        if ctx.engine_state == EngineState::Running {
            result(
                self.name(),
                CheckStatus::Pass,
                "драйвер WinDivert загружен нашим движком",
                details,
                None,
            )
        } else {
            result_fix(
                self.name(),
                CheckStatus::Warn,
                "драйвер WinDivert активен, но движок MXDS не работает",
                details,
                Some(
                    "возможен остаточный драйвер или сторонний bypass; \
                     остаток можно снять кнопкой «Исправить»"
                        .into(),
                ),
                super::FixId::CleanupWinDivert,
            )
        }
    }
}

// --- TCP timestamps ---

struct TcpTimestampsCheck;

impl Check for TcpTimestampsCheck {
    fn name(&self) -> &str {
        "tcp_timestamps"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        match integration::tcp_timestamps_enabled() {
            Ok(true) => result(
                self.name(),
                CheckStatus::Pass,
                "TCP timestamps включены",
                None,
                None,
            ),
            Ok(false) => result_fix(
                self.name(),
                CheckStatus::Warn,
                "TCP timestamps выключены",
                Some("часть стратегий использует timestamps и без них не работает".into()),
                Some(
                    "нажмите «Исправить» или выполните от администратора: \
                     netsh interface tcp set global timestamps=enabled"
                        .into(),
                ),
                super::FixId::EnableTcpTimestamps,
            ),
            Err(e) => result(
                self.name(),
                CheckStatus::Warn,
                "не удалось проверить TCP timestamps",
                Some(e.to_string()),
                None,
            ),
        }
    }
}

// --- Системный прокси ---

struct SystemProxyCheck;

impl Check for SystemProxyCheck {
    fn name(&self) -> &str {
        "system_proxy"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        match integration::system_proxy() {
            Some(p) => result(
                self.name(),
                CheckStatus::Warn,
                "системный прокси включён",
                Some(p.server),
                Some(
                    "прокси может свидетельствовать о VPN — отключите на время диагностики".into(),
                ),
            ),
            _ => result(
                self.name(),
                CheckStatus::Pass,
                "системный прокси не используется",
                None,
                None,
            ),
        }
    }
}

// --- Secure DNS (DoH) ---

struct SecureDnsCheck;

impl Check for SecureDnsCheck {
    fn name(&self) -> &str {
        "secure_dns"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        if integration::secure_dns_configured() {
            result(
                self.name(),
                CheckStatus::Pass,
                "Secure DNS (DoH) настроен в Windows",
                None,
                None,
            )
        } else {
            result_fix(
                self.name(),
                CheckStatus::Warn,
                "Secure DNS не настроен",
                None,
                Some(
                    "нажмите «Исправить» — MXDS зарегистрирует DoH-шаблоны \
                     Cloudflare/Google/Quad9; в Windows 11 можно также включить \
                     зашифрованный DNS в Параметрах сети"
                        .into(),
                ),
                super::FixId::EnableSecureDns,
            )
        }
    }
}

// --- Конфликтующие службы (bypass-аналоги) ---

struct ConflictingServicesCheck;

impl Check for ConflictingServicesCheck {
    fn name(&self) -> &str {
        "conflicting_services"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        let found = components::conflicting_services();
        if found.is_empty() {
            result(
                self.name(),
                CheckStatus::Pass,
                "конфликтующие службы не найдены",
                None,
                None,
            )
        } else {
            result_fix(
                self.name(),
                CheckStatus::Fail,
                "найдены конфликтующие службы",
                Some(found.join(", ")),
                Some(
                    "нажмите «Исправить» — MXDS остановит и удалит их; \
                     одновременная работа двух bypass-средств ломает оба"
                        .into(),
                ),
                super::FixId::RemoveConflictingServices,
            )
        }
    }
}

// --- Стороннее ПО по именам служб (substring, как в оригинале) ---

/// Проверка присутствия службы по подстрокам имени.
/// mode All: все подстроки должны встретиться в одном имени (как цепочка
/// findstr в service.bat); Any: достаточно одной.
struct ServicePatternCheck {
    name: &'static str,
    label: &'static str,
    patterns: &'static [&'static str],
    match_all: bool,
    /// Статус при обнаружении.
    found_status: CheckStatus,
    recommendation: &'static str,
}

impl ServicePatternCheck {
    const ADGUARD: Self = Self {
        name: "adguard",
        label: "Adguard",
        patterns: &["adguard"],
        match_all: false,
        found_status: CheckStatus::Fail,
        recommendation: "Adguard конфликтует с MXDS (Discord) — отключите фильтрацию или Adguard",
    };
    const KILLER: Self = Self {
        name: "killer",
        label: "Killer",
        patterns: &["killer"],
        match_all: false,
        found_status: CheckStatus::Fail,
        recommendation: "службы Killer конфликтуют с MXDS — отключите Killer Control Center",
    };
    const INTEL_CONNECTIVITY: Self = Self {
        name: "intel_connectivity",
        label: "Intel Connectivity Network Service",
        patterns: &["intel", "connectivity", "network"],
        match_all: true,
        found_status: CheckStatus::Fail,
        recommendation: "Intel Connectivity Network Service конфликтует с MXDS — отключите его",
    };
    const CHECK_POINT: Self = Self {
        name: "check_point",
        label: "Check Point",
        patterns: &["tracsrvwrapper", "epwd"],
        match_all: false,
        found_status: CheckStatus::Fail,
        recommendation: "Check Point конфликтует с MXDS — удалите Check Point",
    };
    const SMARTBYTE: Self = Self {
        name: "smartbyte",
        label: "SmartByte",
        patterns: &["smartbyte"],
        match_all: false,
        found_status: CheckStatus::Fail,
        recommendation: "SmartByte конфликтует с MXDS — удалите или отключите через services.msc",
    };
    const VPN: Self = Self {
        name: "vpn_services",
        label: "VPN",
        patterns: &["vpn", "hapservice"],
        match_all: false,
        found_status: CheckStatus::Warn,
        recommendation: "некоторые VPN конфликтуют с MXDS — убедитесь, что все VPN отключены",
    };

    fn find_matches(&self, services: &[String]) -> Vec<String> {
        services
            .iter()
            .filter(|s| {
                let lower = s.to_lowercase();
                if self.match_all {
                    self.patterns.iter().all(|p| lower.contains(p))
                } else {
                    self.patterns.iter().any(|p| lower.contains(p))
                }
            })
            .cloned()
            .collect()
    }
}

impl Check for ServicePatternCheck {
    fn name(&self) -> &str {
        self.name
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        match components::enumerate_services() {
            Ok(services) => {
                let found = self.find_matches(&services);
                if found.is_empty() {
                    result(
                        self.name(),
                        CheckStatus::Pass,
                        format!("{}: не найден", self.label),
                        None,
                        None,
                    )
                } else {
                    result(
                        self.name(),
                        self.found_status,
                        format!("{}: найдены службы", self.label),
                        Some(found.join(", ")),
                        Some(self.recommendation.into()),
                    )
                }
            }
            Err(e) => result(
                self.name(),
                CheckStatus::Warn,
                format!("{}: не удалось перечислить службы", self.label),
                Some(e.to_string()),
                None,
            ),
        }
    }
}

// --- Путь установки ---

struct CyrillicPathCheck;

impl Check for CyrillicPathCheck {
    fn name(&self) -> &str {
        "cyrillic_path"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        if integration::path_has_cyrillic(&ctx.root) {
            result(
                self.name(),
                CheckStatus::Warn,
                "путь установки содержит кириллицу",
                Some(ctx.root.display().to_string()),
                Some("если обход не работает — перенесите MXDS, например в C:\\mxds".into()),
            )
        } else {
            result(
                self.name(),
                CheckStatus::Pass,
                "кириллицы в пути установки нет",
                None,
                None,
            )
        }
    }
}

struct OneDrivePathCheck;

impl Check for OneDrivePathCheck {
    fn name(&self) -> &str {
        "onedrive_path"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        if integration::is_onedrive_path(&ctx.root) {
            result(
                self.name(),
                CheckStatus::Warn,
                "MXDS установлен в папке OneDrive",
                Some(ctx.root.display().to_string()),
                Some("если обход не работает — перенесите MXDS, например в C:\\mxds".into()),
            )
        } else {
            result(
                self.name(),
                CheckStatus::Pass,
                "путь установки вне OneDrive",
                None,
                None,
            )
        }
    }
}

// --- hosts ---

struct HostsFileCheck;

impl HostsFileCheck {
    fn hosts_path() -> PathBuf {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        Path::new(&root).join(r"System32\drivers\etc\hosts")
    }

    /// Ищет записи youtube в содержимом hosts (как оригинал: findstr /I).
    fn youtube_in_hosts(content: &str) -> bool {
        let lower = content.to_lowercase();
        lower.contains("youtube.com") || lower.contains("youtu.be")
    }
}

impl Check for HostsFileCheck {
    fn name(&self) -> &str {
        "hosts_file"
    }

    fn run(&self, _ctx: &DiagContext) -> CheckResult {
        let path = Self::hosts_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if Self::youtube_in_hosts(&content) {
                    result(
                        self.name(),
                        CheckStatus::Warn,
                        "в hosts найдены записи youtube.com / youtu.be",
                        Some(format!("файл: {}", path.display())),
                        Some("такие записи могут ломать доступ к YouTube — удалите их".into()),
                    )
                } else {
                    result(
                        self.name(),
                        CheckStatus::Pass,
                        "записей youtube в hosts нет",
                        None,
                        None,
                    )
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => result(
                self.name(),
                CheckStatus::Pass,
                "файл hosts отсутствует",
                None,
                None,
            ),
            Err(e) => result(
                self.name(),
                CheckStatus::Warn,
                "не удалось прочитать hosts",
                Some(e.to_string()),
                None,
            ),
        }
    }
}

// --- Целостность конфигурации ---

struct ConfigIntegrityCheck;

impl Check for ConfigIntegrityCheck {
    fn name(&self) -> &str {
        "config_integrity"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        let errors = ctx
            .config_issues
            .iter()
            .filter(|i| i.severity == IssueSeverity::Error)
            .count();
        let warnings = ctx.config_issues.len() - errors;
        let details = if ctx.config_issues.is_empty() {
            None
        } else {
            Some(
                ctx.config_issues
                    .iter()
                    .map(|i| format!("{}: {}", i.field, i.message))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        if errors > 0 {
            result(
                self.name(),
                CheckStatus::Fail,
                format!("конфигурация содержит ошибки: {errors}"),
                details,
                Some("исправьте config/config.json или удалите его для сброса к default".into()),
            )
        } else if warnings > 0 {
            result(
                self.name(),
                CheckStatus::Warn,
                format!("конфигурация содержит предупреждения: {warnings}"),
                details,
                None,
            )
        } else {
            result(
                self.name(),
                CheckStatus::Pass,
                "конфигурация валидна",
                None,
                None,
            )
        }
    }
}

// --- Состояние движка ---

struct EngineStateCheck;

impl Check for EngineStateCheck {
    fn name(&self) -> &str {
        "engine_state"
    }

    fn run(&self, ctx: &DiagContext) -> CheckResult {
        match ctx.engine_state {
            EngineState::Running => result(
                self.name(),
                CheckStatus::Pass,
                "движок работает",
                None,
                None,
            ),
            EngineState::Stopped => result(
                self.name(),
                CheckStatus::Pass,
                "движок остановлен",
                Some("ожидаемо, если обход сейчас не нужен".into()),
                None,
            ),
            EngineState::Starting | EngineState::Stopping => result(
                self.name(),
                CheckStatus::Warn,
                "движок в переходном состоянии",
                Some(format!("{:?}", ctx.engine_state)),
                None,
            ),
            EngineState::Failed => result(
                self.name(),
                CheckStatus::Fail,
                "движок в состоянии ошибки",
                ctx.engine_error.clone(),
                Some("см. logs/mxds.log; проверьте конфликтующие службы и права".into()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standalone_ctx() -> DiagContext {
        DiagContext::standalone(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn default_checks_unique_nonempty_names() {
        let checks = default_checks();
        assert!(checks.len() >= 15);
        let mut names: Vec<_> = checks.iter().map(|c| c.name()).collect();
        assert!(names.iter().all(|n| !n.is_empty()));
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            checks.len(),
            "имена проверок должны быть уникальны"
        );
    }

    #[test]
    fn run_all_returns_one_result_per_check() {
        let ctx = standalone_ctx();
        let n = default_checks().len();
        let results = run_all(&ctx, default_checks());
        assert_eq!(results.len(), n);
        // На реальной системе проверки не должны падать паникой.
        for r in &results {
            assert_ne!(r.description, "внутренняя ошибка проверки", "{}", r.name);
        }
    }

    #[test]
    fn run_all_isolates_panics() {
        struct PanicCheck;
        impl Check for PanicCheck {
            fn name(&self) -> &str {
                "panic"
            }
            fn run(&self, _ctx: &DiagContext) -> CheckResult {
                panic!("boom");
            }
        }
        let ctx = standalone_ctx();
        let results = run_all(&ctx, vec![Box::new(PanicCheck), Box::new(ElevationCheck)]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, CheckStatus::Fail);
        assert!(results[0].details.as_deref().unwrap().contains("паникой"));
        // вторая проверка выполнилась
        assert_eq!(results[1].name, "elevation");
    }

    #[test]
    fn windivert_files_present_and_hashes_match() {
        // assets/windivert закоммичены (D-004) — на dev-машине Pass.
        let ctx = standalone_ctx();
        let r = WindivertFilesCheck.run(&ctx);
        assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.details);
    }

    #[test]
    fn windivert_files_missing_dll_fails() {
        let mut ctx = standalone_ctx();
        ctx.dll_path = PathBuf::from("C:\\no\\such\\WinDivert.dll");
        let r = WindivertFilesCheck.run(&ctx);
        assert_eq!(r.status, CheckStatus::Fail);
    }

    #[test]
    fn hosts_youtube_detection() {
        assert!(HostsFileCheck::youtube_in_hosts("127.0.0.1 youtube.com"));
        assert!(HostsFileCheck::youtube_in_hosts("127.0.0.1 YOUTU.BE"));
        assert!(!HostsFileCheck::youtube_in_hosts("127.0.0.1 example.com"));
        assert!(!HostsFileCheck::youtube_in_hosts(""));
    }

    #[test]
    fn service_pattern_matching() {
        let services = vec![
            "KillerNetworkService".to_string(),
            "BFE".to_string(),
            "IntelConnectivityNetworkService".to_string(),
        ];
        assert_eq!(ServicePatternCheck::KILLER.find_matches(&services).len(), 1);
        assert_eq!(
            ServicePatternCheck::INTEL_CONNECTIVITY
                .find_matches(&services)
                .len(),
            1
        );
        assert!(ServicePatternCheck::ADGUARD
            .find_matches(&services)
            .is_empty());
        assert!(ServicePatternCheck::SMARTBYTE
            .find_matches(&services)
            .is_empty());
    }

    #[test]
    fn config_integrity_statuses() {
        use crate::strategy::validation::ValidationIssue;
        let mut ctx = standalone_ctx();
        assert_eq!(ConfigIntegrityCheck.run(&ctx).status, CheckStatus::Pass);

        ctx.config_issues.push(ValidationIssue {
            field: "engine.queue_size_kb".into(),
            message: "вне диапазона".into(),
            severity: IssueSeverity::Error,
        });
        assert_eq!(ConfigIntegrityCheck.run(&ctx).status, CheckStatus::Fail);

        ctx.config_issues[0].severity = IssueSeverity::Warning;
        assert_eq!(ConfigIntegrityCheck.run(&ctx).status, CheckStatus::Warn);
    }

    #[test]
    fn engine_state_statuses() {
        let mut ctx = standalone_ctx();
        assert_eq!(EngineStateCheck.run(&ctx).status, CheckStatus::Pass);

        ctx.engine_state = EngineState::Failed;
        ctx.engine_error = Some("driver error".into());
        let r = EngineStateCheck.run(&ctx);
        assert_eq!(r.status, CheckStatus::Fail);
        assert_eq!(r.details.as_deref(), Some("driver error"));

        ctx.engine_state = EngineState::Starting;
        assert_eq!(EngineStateCheck.run(&ctx).status, CheckStatus::Warn);
    }

    #[test]
    fn enumerate_services_works() {
        // На любой Windows-машине службы есть (BFE как минимум).
        let services = components::enumerate_services().unwrap();
        assert!(!services.is_empty());
        assert!(services.iter().any(|s| s == "BFE"));
    }
}
