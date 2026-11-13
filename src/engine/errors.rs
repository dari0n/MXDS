//! Ошибки движка. См. docs/ERROR_HANDLING.md п.3.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("движок не запущен")]
    NotRunning,
    #[error("движок уже запущен")]
    AlreadyRunning,
    #[error("windivert: {0}")]
    Divert(#[from] crate::windivert::DivertError),
    #[error("рабочий поток движка аварийно завершён: {0}")]
    WorkerCrashed(String),
    #[error("переполнение очереди пакетов, потеряно: {dropped}")]
    QueueOverflow { dropped: u64 },
    #[error("профиль стратегии невалиден: {0}")]
    InvalidProfile(String),
}
