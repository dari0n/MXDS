//! Хранение конфигурации: чтение с восстановлением, атомарная запись,
//! резервные копии, миграции схемы.

use std::path::{Path, PathBuf};

use crate::config::manager::Config;
use crate::config::ConfigError;
use crate::storage::repository::atomic_write;

/// Путь резервной копии: `<file>.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".bak");
    PathBuf::from(p)
}

/// Читает JSON-файл.
pub fn read_json(path: &Path) -> Result<serde_json::Value, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Атомарно сохраняет конфигурацию; перед перезаписью существующего
/// файла создаёт .bak.
pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if path.exists() {
        let bak = backup_path(path);
        std::fs::copy(path, &bak).map_err(|_| ConfigError::BackupFailed)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(config)?;
    atomic_write(path, text.as_bytes()).map_err(|e| match e {
        crate::storage::StorageError::Io(io) => ConfigError::Io(io),
        crate::storage::StorageError::Serialize(se) => ConfigError::Parse(se),
    })?;
    Ok(())
}

/// Миграция схемы конфигурации. Схема 1 — текущая; миграций пока нет,
/// механизм задан (PROMPT.md Цикл 5: версионирование схемы, миграции).
pub fn migrate(config: Config) -> Result<Config, ConfigError> {
    let target = crate::CONFIG_SCHEMA_VERSION;
    if config.schema_version >= target {
        return Ok(config);
    }
    // Будущие миграции: 1 => migrate_1_to_2(config)?, цепочкой до target.
    Err(ConfigError::MigrationFailed {
        from: config.schema_version,
        to: target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_creates_backup_of_previous() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        let mut cfg = Config::default();
        save(&file, &cfg).unwrap();
        cfg.check_updates = true;
        save(&file, &cfg).unwrap();

        let bak = read_json(&backup_path(&file)).unwrap();
        assert_eq!(bak["check_updates"], serde_json::json!(false));
        let cur = read_json(&file).unwrap();
        assert_eq!(cur["check_updates"], serde_json::json!(true));
    }

    #[test]
    fn read_json_of_backup_recovers_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        std::fs::write(backup_path(&file), r#"{"schema_version":1}"#).unwrap();
        let v = read_json(&backup_path(&file)).unwrap();
        assert_eq!(v["schema_version"], serde_json::json!(1));
    }

    #[test]
    fn migrate_identity_for_current_schema() {
        let cfg = Config::default();
        let cfg = migrate(cfg).unwrap();
        assert_eq!(cfg.schema_version, crate::CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_unknown_old_schema_fails() {
        let cfg = Config {
            schema_version: 0,
            ..Default::default()
        };
        assert!(matches!(
            migrate(cfg),
            Err(ConfigError::MigrationFailed { .. })
        ));
    }
}
