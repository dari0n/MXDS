//! Имена и разбор ENV-переменных (префикс MXDS_*, D-007).
//! ENV применяется один раз при старте (PROMPT.md, раздел 8).

use crate::logging::LogLevel;

pub const MXDS_LOG_LEVEL: &str = "MXDS_LOG_LEVEL";
pub const MXDS_CONFIG_PATH: &str = "MXDS_CONFIG_PATH";
pub const MXDS_DATA_PATH: &str = "MXDS_DATA_PATH";
pub const MXDS_CHECK_UPDATES: &str = "MXDS_CHECK_UPDATES";

/// Разбор значения MXDS_LOG_LEVEL. Некорректное значение — None
/// (вызывающий код логирует WARN и оставляет конфиг без изменений).
pub fn parse_log_level(value: &str) -> Option<LogLevel> {
    match value.to_ascii_lowercase().as_str() {
        "error" => Some(LogLevel::Error),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

/// Разбор булева ENV ("true/false", "1/0", "yes/no", регистр не важен).
pub fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_parsing() {
        assert_eq!(parse_log_level("INFO"), Some(LogLevel::Info));
        assert_eq!(parse_log_level("debug"), Some(LogLevel::Debug));
        assert_eq!(parse_log_level("nonsense"), None);
    }

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("2"), None);
    }
}
