//! Ошибки Windows-слоя. См. docs/ERROR_HANDLING.md п.3.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WindowsError {
    #[error("операция требует прав администратора")]
    NotElevated,
    #[error("неподдерживаемая версия ОС: build {build}")]
    OsUnsupported { build: u32 },
    #[error("служба {name}: операция {op} завершилась кодом {code}")]
    Service {
        op: &'static str,
        name: String,
        code: u32,
    },
    #[error("реестр {key}: код {code}")]
    Registry { key: String, code: u32 },
    #[error("netsh: операция завершилась ошибкой")]
    NetshFailed,
    #[error("ipc: {0}")]
    Ipc(String),
    #[error("другой экземпляр MXDS уже запущен")]
    InstanceAlreadyRunning,
    #[error("неизвестная ошибка Windows (код {0})")]
    Unknown(u32),
}
