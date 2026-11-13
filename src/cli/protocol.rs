//! Протокол Named Pipe IPC (D-009): JSON-lines, allowlist команд.
//! Произвольное выполнение команд ОС отсутствует по построению —
//! сервер принимает только перечисленные варианты IpcCommand.

use serde::{Deserialize, Serialize};

/// Команды, принимаемые IPC-сервером работающего экземпляра.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcCommand {
    /// Снимок состояния (движок, профиль, последние ошибки).
    Status,
    /// Остановить движок (приложение продолжает работать).
    Stop,
    /// Запустить движок с активным профилем.
    Run,
    /// Текущая конфигурация.
    GetConfig,
    /// Последние N строк журнала.
    LogsTail { lines: u32 },
}

/// Ответ сервера: либо data, либо error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// Сериализация команды в строку протокола.
pub fn encode_command(cmd: &IpcCommand) -> String {
    // serde_json не падает на перечислении без ссылок.
    serde_json::to_string(cmd).expect("IpcCommand сериализуется")
}

/// Разбор команды; неизвестные/битые команды отклоняются.
pub fn decode_command(line: &str) -> Result<IpcCommand, String> {
    serde_json::from_str(line).map_err(|e| format!("невалидная команда IPC: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip() {
        for cmd in [
            IpcCommand::Status,
            IpcCommand::Stop,
            IpcCommand::Run,
            IpcCommand::GetConfig,
            IpcCommand::LogsTail { lines: 100 },
        ] {
            let line = encode_command(&cmd);
            assert!(!line.contains('\n'), "протокол — одна строка");
            assert_eq!(decode_command(&line).unwrap(), cmd);
        }
    }

    #[test]
    fn unknown_command_rejected() {
        assert!(decode_command(r#"{"cmd":"exec","args":["format c:"]}"#).is_err());
        assert!(decode_command("not json").is_err());
        assert!(decode_command("").is_err());
    }

    #[test]
    fn response_serialization() {
        let ok = IpcResponse::ok(serde_json::json!({"engine": "Running"}));
        let s = serde_json::to_string(&ok).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(!s.contains("error"));
        let err = IpcResponse::err("нет прав");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"ok\":false"));
        assert!(s.contains("нет прав"));
    }
}
