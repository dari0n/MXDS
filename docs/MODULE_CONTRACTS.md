# Контракты модулей MXDS

Дата: 2026-11-13. Статус: документ Цикла 3.
Сигнатуры — проектные (Rust-псевдокод); уточняются при реализации,
расхождения фиксируются в DECISIONS.md.

## 1. Типы-основы (shared)

```rust
pub enum EngineState { Stopped, Starting, Running, Stopping, Failed }
pub enum CheckStatus { Pass, Warn, Fail, Blocked }
pub enum CompatStatus { Supported, PartiallySupported, Unsupported, NotTested }

pub struct CheckResult {
    pub name: String, pub status: CheckStatus, pub description: String,
    pub duration: Duration, pub details: Option<String>,
    pub recommendation: Option<String>,
}

pub enum AppEvent {            // Event Bus (PROMPT.md п.7)
    ApplicationStarted, ApplicationStopped,
    EngineStarted, EngineStopped, EngineFailed(String),
    ProfileChanged(ProfileId), SettingsChanged,
    ComponentStateChanged(ComponentId, String),
    DiagnosticCompleted(Vec<CheckResult>),
    ConnectivityCheckCompleted(Vec<ConnectivityResult>),
}
```

## 2. Application Core (`application/`)

Публичный API — единственная точка входа для GUI и CLI.

```rust
pub struct CoreManager { /* владеет всеми менеджерами */ }

impl CoreManager {
    pub async fn bootstrap(paths: AppPaths) -> Result<Self, CoreError>;
    pub async fn state(&self) -> AppState;              // engine state, active profile, last errors
    pub async fn start_engine(&self) -> Result<(), EngineError>;
    pub async fn stop_engine(&self) -> Result<(), EngineError>;
    pub async fn list_profiles(&self) -> Vec<ProfileInfo>;
    pub async fn select_profile(&self, id: ProfileId) -> Result<(), StrategyError>;
    pub async fn config(&self) -> Config;
    pub async fn update_config(&self, patch: ConfigPatch) -> Result<(), ConfigError>;
    pub async fn run_diagnostics(&self) -> Vec<CheckResult>;
    pub async fn check_connectivity(&self, targets: Option<Vec<Target>>) -> Vec<ConnectivityResult>;
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent>;
    pub async fn shutdown(&self) -> Result<(), CoreError>;
}
```

Гарантии: все методы идемпотентны к повторному вызову; `start_engine` при
RUNNING — Ok(()) (no-op); `stop_engine` при STOPPED — Ok(()).
Состояние возвращается из фактических источников (EngineHandle::is_alive).

## 3. Engine (`engine/`)

```rust
pub struct EngineManager;
impl EngineManager {
    pub fn start(&mut self, profile: &ResolvedProfile, cfg: &EngineConfig)
        -> Result<EngineHandle, EngineError>;
    pub fn stop(&mut self) -> Result<(), EngineError>;       // graceful: Shutdown→drain→Close
    pub fn state(&self) -> EngineState;                      // по факту: хендл + живой цикл
    pub fn stats(&self) -> EngineStats;                      // packets recv/sent/dropped
}
```

- Рабочий цикл: выделенный OS-поток; Recv → match profile rules →
  Send (оригинал/фейки/сплиты) → статистика.
- `ResolvedProfile` — провалидированная стратегия с разрешёнными путями
  payload'ов и сгенерированной строкой фильтра WinDivert.
- Движок не читает конфиг и не знает о GUI/CLI: только входные параметры.

## 4. WinDivert Adapter (`windivert/`)

```rust
pub struct DivertHandle { /* NonNull, Drop → WinDivertClose */ }
impl DivertHandle {
    pub fn open(filter: &str) -> Result<Self, DivertError>;   // layer NETWORK, priority 0
    pub fn set_param(&self, p: DivertParam, v: u64) -> Result<(), DivertError>;
    pub fn driver_version(&self) -> Result<(u64, u64), DivertError>;
    pub fn recv(&self, buf: &mut [u8]) -> Result<(usize, Address), DivertError>;
    pub fn send(&self, pkt: &[u8], addr: &Address) -> Result<usize, DivertError>;
    pub fn shutdown(&self) -> Result<(), DivertError>;
}
```

- `DivertError`: typed variants для кодов 2/5/87/577/654/1060/1275/1753 +
  `Unknown(u32)`.
- Маппинг стратегий (fake/split/seqovl/fooling) живёт в `engine/`,
  адаптер знает только пакеты и адреса.

## 5. Windows Integration (`windows/`)

```rust
pub fn is_elevated() -> bool;
pub fn os_version() -> OsVersion;                       // build, ubr, display
pub fn bfe_running() -> Result<bool, WindowsError>;
pub fn tcp_timestamps_enabled() -> Result<bool, WindowsError>;
pub fn enable_tcp_timestamps() -> Result<(), WindowsError>;  // только по подтверждению
pub fn system_proxy() -> Option<ProxyInfo>;
pub fn secure_dns_configured() -> bool;
pub fn path_has_cyrillic(p: &Path) -> bool;
pub fn is_onedrive_path(p: &Path) -> bool;

pub struct ServiceManager;   // SCM: install/remove/start/stop/query "mxds",
                             // stop/remove WinDivert* — всё typed Result
pub struct SingleInstance;   // мьютекс + lock-файл; Drop освобождает
pub struct IpcServer / IpcClient;  // Named Pipe MXDS.Control, DACL user-only
pub fn conflicting_services() -> Vec<String>;  // GoodbyeDPI, winws*, zapret, ...
```

## 6. Strategy Manager (`strategy/`)

```rust
pub struct Profile { pub id: ProfileId, pub meta: ProfileMeta,
                     pub rules: Vec<ProfileRule>, pub compat: CompatStatus }
pub struct ProfileMeta { pub name: String, pub description: String,
                         pub version: String, pub origin: Option<String> }

impl StrategyManager {
    pub fn discover(dir: &Path) -> Result<Vec<Profile>, StrategyError>;
    pub fn validate(p: &Profile, payloads: &Path) -> Vec<ValidationIssue>;
    pub fn import_winws_args(name: &str, args: &str) -> Result<Profile, StrategyError>;
    pub fn resolve(p: &Profile, cfg: &Config) -> Result<ResolvedProfile, StrategyError>;
}
```

- Импорт из bat-аргументов — один раз, на этапе миграции данных (скрипт
  Цикла 8), не в рантайме.
- Профили без подтверждённого теста: compat = NotTested (PROMPT.md, Цикл 8).

## 7. Config Manager (`config/`)

```rust
pub struct Config { pub schema_version: u32, pub log_level: Level,
                    pub active_profile: Option<ProfileId>,
                    pub game_filter: GameFilter, pub ipset_mode: IpsetMode,
                    pub check_updates: bool, pub engine: EngineConfig, /* ... */ }

impl ConfigManager {
    pub fn load(paths: &ConfigPaths) -> Result<Self, ConfigError>; // default → json → ENV
    pub fn validate(cfg: &Config) -> Vec<ValidationIssue>;
    pub fn save(&self, cfg: &Config) -> Result<(), ConfigError>;   // temp+rename, .bak
    pub fn migrate(cfg: Config, from: u32) -> Result<Config, ConfigError>;
}
```

- ENV-префикс `MXDS_*` (соответствует духу `ZAPRET_*` из PROMPT.md,
  переименование зафиксировано в D-007): `MXDS_LOG_LEVEL`,
  `MXDS_CONFIG_PATH`, `MXDS_DATA_PATH`, `MXDS_CHECK_UPDATES`.
- ENV применяется один раз при старте; runtime-изменение ENV не
  отслеживается (PROMPT.md, раздел 8).

## 8. Diagnostics (`diagnostics/`)

```rust
pub trait Check { fn name(&self) -> &str;
                  fn run(&self, ctx: &DiagContext) -> CheckResult; }
pub fn default_checks() -> Vec<Box<dyn Check>>;  // состав — Цикл 10,
                                                 // по FUNCTIONALITY_MAP F-11
```

## 9. Connectivity (`connectivity/`)

```rust
pub struct Target { pub name: String, pub url: Url }  // или Ping(IpAddr)
pub struct ConnectivityResult { pub target: String, pub stage: Stage,
    pub status: CheckStatus, pub elapsed: Duration, pub detail: String }
pub enum Stage { Dns, Tcp, Tls, Http, Ping }

pub async fn check(targets: &[Target], timeout: Duration) -> Vec<ConnectivityResult>;
```

- Каждая цель проверяется каскадом DNS→TCP→TLS→HTTP; провал этапа
  фиксируется, вышестоящие этапы помечаются Blocked.
- История: append-only JSON-lines в `data/connectivity-history.jsonl`,
  ротация по размеру.

## 10. CLI (`cli/`) и GUI (`gui/`)

- CLI: clap derive; команды PROMPT.md Цикл 12 (с префиксом `mxds`);
  коды выхода: 0 ok, 1 ошибка выполнения, 2 ошибка аргументов (clap),
  3 диагностика FAIL, 4 нет прав.
- CLI сначала пробует IpcClient к работающему экземпляру (status/stop);
  при отсутствии — автономный Core в том же процессе.
- GUI: egui/eframe, экраны PROMPT.md Цикл 13; опрос состояния +
  подписка на Event Bus; никаких прямых вызовов windows/windivert.

## 11. Logging (`logging/`)

- `tracing` + `tracing-subscriber`: fmt-layer в файл `logs/mxds.log`
  (ротация daily + max size) и в GUI/CLI-канал; уровень из конфига/ENV.
- Секреты/содержимое пакетов не логируются (SECURITY_REVIEW п.5).

## 12. Storage (`storage/`)

- Атомарная запись (temp + rename), append-only журналы, простые
  key-value JSON-файлы состояния. Без БД.
