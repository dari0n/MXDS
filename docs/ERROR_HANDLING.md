# Обработка ошибок MXDS

Дата: 2026-11-13. Статус: документ Цикла 3.

## 1. Принципы

1. `Result` + `thiserror` везде; `unwrap`/`expect` запрещены вне тестов
   (контроль clippy-линтами в Цикле 4).
2. Ни один системный вызов не приводит к панике.
3. Ошибка = тип + контекст операции + рекомендация пользователю.
4. Ложного успеха нет: состояние подтверждается фактами
   (PROMPT.md, раздел 6).
5. Ошибки не скрываются: провалы тестов и диагностик фиксируются в
   context.md и ACCEPTANCE_REPORT.

## 2. Иерархия ошибок

```rust
#[derive(thiserror::Error, Debug)]
pub enum MxdsError {
    #[error("config: {0}")]        Config(#[from] ConfigError),
    #[error("strategy: {0}")]      Strategy(#[from] StrategyError),
    #[error("engine: {0}")]        Engine(#[from] EngineError),
    #[error("divert: {0}")]        Divert(#[from] DivertError),
    #[error("windows: {0}")]       Windows(#[from] WindowsError),
    #[error("connectivity: {0}")]  Connectivity(#[from] ConnectivityError),
    #[error("storage: {0}")]       Storage(#[from] StorageError),
}
```

Каждая доменная ошибка несёт: машинный вариант, человекочитаемое
сообщение (ru), опциональную рекомендацию.

## 3. Доменные варианты (проектные)

### DivertError
`DriverNotFound` (2), `AccessDenied` (5), `InvalidFilter` (87),
`DriverSignatureInvalid` (577), `DriverVersionConflict` (654),
`DriverBlocked` (1275), `BfeNotRunning` (1753), `VersionMismatch {
expected, got }`, `Recv/Send/Io(u32)`, `Unknown(u32)`.

### EngineError
`NotRunning`, `AlreadyRunning`, `Divert(DivertError)`,
`WorkerCrashed(String)`, `QueueOverflow { dropped: u64 }`,
`InvalidProfile(ValidationIssue)`.

### ConfigError
`Io`, `Parse { line, col }`, `Validation(Vec<ValidationIssue>)`,
`SchemaTooNew { file, supported }`, `MigrationFailed { from, to }`,
`BackupFailed`.

### WindowsError
`NotElevated`, `OsUnsupported { build }`, `Service(ServiceError {
op, name, code })`, `Registry { key, code }`, `NetshFailed`,
`Ipc(IpcError)`, `InstanceAlreadyRunning`, `Unknown(u32)`.

### StrategyError
`NotFound(ProfileId)`, `Parse { file, detail }`, `ValidationFailed`,
`PayloadMissing { file, sha256_expected }`, `ImportUnsupported(String)`.

### ConnectivityError
`Dns(String)`, `Tcp(io::Error)`, `Tls(String)`, `Http { status }`,
`Timeout { stage, elapsed }`, `NoInternet`.

## 4. Маппинг системных ошибок

- WinAPI: `GetLastError()` захватывается СРАЗУ после неудачного вызова
  (до любого другого FFI), оборачивается с именем операции
  (`WindowsError::Service { op: "CreateService", ... }`).
- WinDivert: таблица кодов — WINDIVERT_INTEGRATION.md, раздел 6;
  маппинг в `DivertError` выполняется в одном месте (`ffi.rs`),
  выше по стеку коды не передаются сырыми.
- Все ошибки логируются на уровне ERROR с контекстом; пользователю в
  GUI/CLI показывается сообщение + рекомендация.

## 5. Восстановление

| Сбой | Реакция |
|---|---|
| Падение рабочего потока движка | EngineFailed → FAILED; автоперезапуск НЕ выполняется; пользователь решает |
| Переполнение очереди WinDivert | WARN + счётчик dropped; рост → FAIL диагностики с рекомендацией тюнинга queue-параметров |
| Повреждённый config.json | Загрузка .bak → WARN; оба повреждены → default.json + WARN |
| Невалидный активный профиль | Движок не стартует; статус профиля UNSUPPORTED/INVALID в UI |
| Отказ IPC | CLI деградирует в автономный режим с WARN |
| Отказ записи истории/логов | WARN в stderr; работа продолжается |

## 6. Коды завершения CLI

`0` успех; `1` ошибка выполнения; `2` ошибка аргументов;
`3` диагностика содержит FAIL; `4` недостаточно прав.

## 7. Паники

Паника допустима только при нарушении внутренних инвариантов программиста
(не пользовательского ввода). GUI-процесс ставит panic-hook: запись в лог
+ попытка graceful shutdown движка + сообщение пользователю.
