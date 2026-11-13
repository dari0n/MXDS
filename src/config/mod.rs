//! Config Manager — централизованная конфигурация (Цикл 5).
//! См. docs/MODULE_CONTRACTS.md п.7, PROMPT.md Цикл 5 и раздел 8.
//!
//! Цикл 4: типы конфигурации, имена ENV, значения по умолчанию.
//! Загрузка/валидация/миграции/атомарная запись — Цикл 5.

pub mod env;
pub mod loader;
pub mod manager;
pub mod repository;
pub mod validation;

pub use loader::{ConfigPaths, LoadOutcome};
pub use manager::{Config, ConfigManager, GameFilter, GameFilterMode, IpsetMode};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("ошибка ввода-вывода конфигурации: {0}")]
    Io(#[from] std::io::Error),
    #[error("ошибка разбора JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("конфигурация не прошла валидацию: {0} проблем")]
    Validation(usize),
    #[error("схема конфигурации {file} новее поддерживаемой {supported}")]
    SchemaTooNew { file: u32, supported: u32 },
    #[error("миграция конфигурации {from} -> {to} не удалась")]
    MigrationFailed { from: u32, to: u32 },
    #[error("не удалось создать резервную копию конфигурации")]
    BackupFailed,
}
