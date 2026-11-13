//! Валидация конфигурации.
//! Включает перенос логики validate_game_filter_range оригинала
//! (service.bat): элементы через запятую, порт 1-65535 или диапазон
//! start-end (start <= end), без пробелов.

use crate::config::manager::Config;
use crate::strategy::validation::{IssueSeverity, ValidationIssue};

/// Проверка строки портов Game Filter: "443", "1024-65535,1936".
/// Перенос validate_game_filter_range из service.bat.
pub fn validate_port_ranges(input: &str) -> bool {
    let input = input.replace(' ', "");
    if input.is_empty() {
        return false;
    }
    input.split(',').all(|item| {
        let (start, end) = match item.split_once('-') {
            Some((a, b)) => (a, b),
            None => (item, item),
        };
        let (Ok(s), Ok(e)) = (start.parse::<u32>(), end.parse::<u32>()) else {
            return false;
        };
        // Оригинал отклоняет числа длиной > 5 цифр; диапазон u32 + проверка
        // границ эквивалентны и строже.
        s >= 1 && e <= 65535 && s <= e
    })
}

/// Полная валидация конфигурации. Пустой результат = конфигурация валидна.
pub fn validate(config: &Config) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if config.schema_version == 0 {
        issues.push(ValidationIssue {
            field: "schema_version".into(),
            message: "версия схемы должна быть >= 1".into(),
            severity: IssueSeverity::Error,
        });
    }

    if !validate_port_ranges(&config.game_filter.tcp_ports) {
        issues.push(ValidationIssue {
            field: "game_filter.tcp_ports".into(),
            message: format!("некорректные порты: {:?}", config.game_filter.tcp_ports),
            severity: IssueSeverity::Error,
        });
    }
    if !validate_port_ranges(&config.game_filter.udp_ports) {
        issues.push(ValidationIssue {
            field: "game_filter.udp_ports".into(),
            message: format!("некорректные порты: {:?}", config.game_filter.udp_ports),
            severity: IssueSeverity::Error,
        });
    }

    // Диапазоны очереди WinDivert (WINDIVERT_INTEGRATION п.4).
    if config.engine.queue_length == 0 || config.engine.queue_length > 65535 {
        issues.push(ValidationIssue {
            field: "engine.queue_length".into(),
            message: "queue_length вне диапазона 1..=65535".into(),
            severity: IssueSeverity::Error,
        });
    }
    if config.engine.queue_time_ms == 0 || config.engine.queue_time_ms > 60_000 {
        issues.push(ValidationIssue {
            field: "engine.queue_time_ms".into(),
            message: "queue_time_ms вне диапазона 1..=60000".into(),
            severity: IssueSeverity::Error,
        });
    }
    if config.engine.queue_size_kb == 0 {
        issues.push(ValidationIssue {
            field: "engine.queue_size_kb".into(),
            message: "queue_size_kb должен быть > 0".into(),
            severity: IssueSeverity::Error,
        });
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ranges_valid() {
        assert!(validate_port_ranges("443"));
        assert!(validate_port_ranges("1024-65535"));
        assert!(validate_port_ranges("80,443,5000-6000"));
        assert!(validate_port_ranges("1"));
        assert!(validate_port_ranges("65535"));
    }

    #[test]
    fn port_ranges_invalid() {
        assert!(!validate_port_ranges(""));
        assert!(!validate_port_ranges("0"));
        assert!(!validate_port_ranges("65536"));
        assert!(!validate_port_ranges("100-50"));
        assert!(!validate_port_ranges("abc"));
        assert!(!validate_port_ranges("80,"));
        assert!(!validate_port_ranges("-100"));
    }

    #[test]
    fn default_config_is_valid() {
        assert!(validate(&Config::default()).is_empty());
    }

    #[test]
    fn invalid_game_filter_detected() {
        let cfg = Config {
            game_filter: crate::config::manager::GameFilter {
                tcp_ports: "70000-80000".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let issues = validate(&cfg);
        assert!(issues
            .iter()
            .any(|i| i.field == "game_filter.tcp_ports" && i.severity == IssueSeverity::Error));
    }
}
