//! Загрузка и слияние конфигурации.
//! Приоритет: значения по умолчанию < default.json < config.json < ENV
//! (PROMPT.md Цикл 5; ENV применяется один раз при старте).

use std::path::Path;

use crate::config::env as cfg_env;
use crate::config::manager::Config;
use crate::config::{repository, ConfigError};

/// Пути конфигурации.
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    /// Эталон из дистрибутива (read-only).
    pub default_path: std::path::PathBuf,
    /// Пользовательская конфигурация.
    pub config_path: std::path::PathBuf,
}

impl ConfigPaths {
    /// Пути по умолчанию рядом с exe с учётом MXDS_CONFIG_PATH.
    pub fn resolve(base_dir: &Path) -> Self {
        let config_path = std::env::var(cfg_env::MXDS_CONFIG_PATH)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| base_dir.join("config").join("config.json"));
        Self {
            default_path: base_dir.join("config").join("default.json"),
            config_path,
        }
    }
}

/// Рекурсивное слияние JSON-объектов: значения `over` побеждают.
pub fn merge_json(base: &mut serde_json::Value, over: serde_json::Value) {
    match (base, over) {
        (serde_json::Value::Object(b), serde_json::Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge_json(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, over) => *slot = over,
    }
}

/// Итог загрузки: конфигурация + предупреждения (деградации).
#[derive(Debug)]
pub struct LoadOutcome {
    pub config: Config,
    pub warnings: Vec<String>,
}

/// Полная загрузка: default.json → config.json (с восстановлением из
/// .bak при повреждении) → ENV.
pub fn load(paths: &ConfigPaths) -> Result<LoadOutcome, ConfigError> {
    let mut warnings = Vec::new();

    // 1. База: встроенные дефолты, перекрытые default.json дистрибутива.
    let mut merged = serde_json::to_value(Config::default())?;
    if paths.default_path.exists() {
        match repository::read_json(&paths.default_path) {
            Ok(v) => merge_json(&mut merged, v),
            Err(e) => {
                warnings.push(format!(
                    "default.json повреждён ({e}); используются встроенные значения"
                ));
            }
        }
    }

    // 2. Пользовательский config.json с восстановлением из .bak.
    if paths.config_path.exists() {
        match repository::read_json(&paths.config_path) {
            Ok(v) => merge_json(&mut merged, v),
            Err(e) => match repository::read_json(&repository::backup_path(&paths.config_path)) {
                Ok(v) => {
                    warnings.push(format!(
                        "config.json повреждён ({e}); конфигурация восстановлена из .bak"
                    ));
                    merge_json(&mut merged, v);
                }
                Err(_) => warnings.push(format!(
                    "config.json и .bak повреждены ({e}); используются значения по умолчанию"
                )),
            },
        }
    }

    let mut config: Config = serde_json::from_value(merged)?;
    check_schema(&config)?;

    // 3. ENV-переопределения (один раз при старте).
    apply_env(&mut config, &mut warnings);

    Ok(LoadOutcome { config, warnings })
}

fn check_schema(config: &Config) -> Result<(), ConfigError> {
    let supported = crate::CONFIG_SCHEMA_VERSION;
    if config.schema_version > supported {
        return Err(ConfigError::SchemaTooNew {
            file: config.schema_version,
            supported,
        });
    }
    if config.schema_version < supported {
        // Миграции: repository::migrate (вызов выполняет ConfigManager).
        tracing::info!(
            from = config.schema_version,
            to = supported,
            "конфигурация будет мигрирована"
        );
    }
    Ok(())
}

fn apply_env(config: &mut Config, warnings: &mut Vec<String>) {
    if let Ok(v) = std::env::var(cfg_env::MXDS_LOG_LEVEL) {
        match cfg_env::parse_log_level(&v) {
            Some(level) => config.log_level = level,
            None => warnings.push(format!(
                "{}: некорректное значение {v:?}, проигнорировано",
                cfg_env::MXDS_LOG_LEVEL
            )),
        }
    }
    if let Ok(v) = std::env::var(cfg_env::MXDS_CHECK_UPDATES) {
        match cfg_env::parse_bool(&v) {
            Some(b) => config.check_updates = b,
            None => warnings.push(format!(
                "{}: некорректное значение {v:?}, проигнорировано",
                cfg_env::MXDS_CHECK_UPDATES
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_json_overwrites_scalars_and_merges_objects() {
        let mut base = serde_json::json!({"a": 1, "b": {"c": 2, "d": 3}});
        merge_json(&mut base, serde_json::json!({"a": 10, "b": {"c": 20}}));
        assert_eq!(base, serde_json::json!({"a": 10, "b": {"c": 20, "d": 3}}));
    }

    #[test]
    fn load_defaults_when_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::resolve(dir.path());
        let outcome = load(&paths).unwrap();
        assert_eq!(outcome.config.schema_version, crate::CONFIG_SCHEMA_VERSION);
        assert!(!outcome.config.check_updates);
    }

    #[test]
    fn user_json_overrides_default_json() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_dir = dir.path().join("config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("default.json"), r#"{"check_updates": false}"#).unwrap();
        std::fs::write(cfg_dir.join("config.json"), r#"{"check_updates": true}"#).unwrap();
        let outcome = load(&ConfigPaths::resolve(dir.path())).unwrap();
        assert!(outcome.config.check_updates);
    }

    #[test]
    fn corrupted_config_falls_back_to_bak() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_dir = dir.path().join("config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("config.json"), b"{ not json").unwrap();
        std::fs::write(
            cfg_dir.join("config.json.bak"),
            r#"{"check_updates": true}"#,
        )
        .unwrap();
        let outcome = load(&ConfigPaths::resolve(dir.path())).unwrap();
        assert!(outcome.config.check_updates);
        assert!(!outcome.warnings.is_empty());
    }

    #[test]
    fn schema_too_new_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_dir = dir.path().join("config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("config.json"), r#"{"schema_version": 999}"#).unwrap();
        let err = load(&ConfigPaths::resolve(dir.path())).unwrap_err();
        assert!(matches!(err, ConfigError::SchemaTooNew { .. }));
    }
}
