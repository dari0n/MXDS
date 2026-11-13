//! Diagnostics — системные и сетевые проверки (Цикл 10).
//! См. docs/MODULE_CONTRACTS.md п.8, PROMPT.md Цикл 10.
//!
//! Состав проверок — по FUNCTIONALITY_MAP F-11 (перенос Run Diagnostics
//! оригинала): права, ОС, BFE, WinDivert (файлы/драйвер), TCP timestamps,
//! прокси, DoH, конфликтующие и сторонние службы, VPN, кириллица/OneDrive
//! в пути, hosts, целостность конфигурации, состояние движка.

pub mod checks;
pub mod fixes;

use std::path::PathBuf;
use std::time::Duration;

use crate::engine::EngineState;
use crate::strategy::validation::ValidationIssue;

pub use checks::{default_checks, run_all};
pub use fixes::run_fix;

/// Статус проверки (PROMPT.md Цикл 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Blocked,
}

/// Авто-исправление, доступное для проверки (кнопка «Исправить» в GUI,
/// `run_fix` в Core). Исправления меняют настройки ОС — требуют прав
/// администратора и сопровождаются понятным сообщением результата.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FixId {
    /// Включить TCP timestamps (netsh).
    EnableTcpTimestamps,
    /// Настроить Secure DNS (DoH-шаблоны для известных резолверов).
    EnableSecureDns,
    /// Снять осиротевший драйвер WinDivert (stop+delete служб).
    CleanupWinDivert,
    /// Остановить и удалить конфликтующие bypass-службы.
    RemoveConflictingServices,
}

/// Структурированный результат одной проверки.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub status: CheckStatus,
    pub description: String,
    #[serde(with = "humantime_ms")]
    pub duration: Duration,
    pub details: Option<String>,
    pub recommendation: Option<String>,
    /// Доступное авто-исправление (если проблема исправима).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixId>,
}

/// Сериализация Duration в миллисекундах (u64) для отчётов.
mod humantime_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

/// Контекст, необходимый проверкам: пути установки и снимок состояния
/// приложения (движок, конфигурация). Собирается CoreManager'ом;
/// для автономного запуска (CLI без Core) — `DiagContext::standalone`.
#[derive(Debug, Clone)]
pub struct DiagContext {
    /// Корень установки MXDS (проверки пути: кириллица, OneDrive).
    pub root: PathBuf,
    /// Путь к WinDivert.dll (рядом ожидается WinDivert64.sys).
    pub dll_path: PathBuf,
    /// Текущее состояние движка.
    pub engine_state: EngineState,
    /// Последняя ошибка движка (если engine_state == Failed).
    pub engine_error: Option<String>,
    /// Проблемы валидации текущей конфигурации.
    pub config_issues: Vec<ValidationIssue>,
}

impl DiagContext {
    /// Контекст без приложения: движок считается остановленным,
    /// конфигурация — валидной.
    pub fn standalone(root: PathBuf) -> Self {
        Self {
            dll_path: crate::windivert::adapter::default_dll_path(),
            root,
            engine_state: EngineState::Stopped,
            engine_error: None,
            config_issues: Vec::new(),
        }
    }
}

/// Контракт одной проверки (MODULE_CONTRACTS п.8).
pub trait Check: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, ctx: &DiagContext) -> CheckResult;
}
