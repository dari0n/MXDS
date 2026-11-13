//! MXDS — самостоятельное Windows-приложение на Rust, переносящее
//! управляющую и сетевую функциональность Flowseal/zapret-discord-youtube.
//!
//! winws.exe и Batch-файлы не используются; WinDivert применяется только
//! как системный сетевой компонент. См. docs/ARCHITECTURE.md.

pub mod application;
pub mod cli;
pub mod config;
pub mod connectivity;
pub mod diagnostics;
pub mod engine;
pub mod gui;
pub mod logging;
pub mod storage;
pub mod strategy;
pub mod windivert;
pub mod windows;

use thiserror::Error;

/// Корневая ошибка приложения: объединяет доменные ошибки модулей.
/// См. docs/ERROR_HANDLING.md, раздел 2.
#[derive(Error, Debug)]
pub enum MxdsError {
    #[error("config: {0}")]
    Config(#[from] config::ConfigError),
    #[error("strategy: {0}")]
    Strategy(#[from] strategy::StrategyError),
    #[error("engine: {0}")]
    Engine(#[from] engine::EngineError),
    #[error("divert: {0}")]
    Divert(#[from] windivert::DivertError),
    #[error("windows: {0}")]
    Windows(#[from] windows::WindowsError),
    #[error("connectivity: {0}")]
    Connectivity(#[from] connectivity::ConnectivityError),
    #[error("storage: {0}")]
    Storage(#[from] storage::StorageError),
}

/// Версия схемы конфигурации, поддерживаемая этой сборкой.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Имя службы Windows и пайпа локальной координации (D-007, D-009).
pub const SERVICE_NAME: &str = "mxds";
pub const IPC_PIPE_NAME: &str = r"\\.\pipe\MXDS.Control";
