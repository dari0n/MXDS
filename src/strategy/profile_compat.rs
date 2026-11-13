//! Персистентные статусы совместимости профилей (feature после Цикла 16).
//!
//! Результаты живых проверок (например, перебора scripts/profile-sweep.ps1)
//! хранятся в `data/profile-compat.json` и накладываются на профили при
//! discover/load — поле `compat` в самих файлах профилей не меняется.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::profile::CompatStatus;

/// Запись о проверке профиля.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatRecord {
    pub status: CompatStatus,
    /// Свободная заметка, например "youtube OK 0.7s, discord RST".
    #[serde(default)]
    pub note: String,
    /// Дата проверки (YYYY-MM-DD).
    #[serde(default)]
    pub tested_at: String,
}

/// Хранилище статусов: id профиля → запись.
#[derive(Debug, Clone, Default)]
pub struct CompatStore {
    map: BTreeMap<String, CompatRecord>,
}

impl CompatStore {
    /// Путь файла хранилища.
    pub fn path(root: &Path) -> PathBuf {
        root.join("data").join("profile-compat.json")
    }

    /// Загрузка; отсутствующий или повреждённый файл → пустое хранилище
    /// (best effort, деградация без паники).
    pub fn load(root: &Path) -> Self {
        let path = Self::path(root);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<BTreeMap<String, CompatRecord>>(&bytes) {
            Ok(map) => Self { map },
            Err(e) => {
                tracing::warn!(error = %e, "profile-compat.json повреждён — игнорируется");
                Self::default()
            }
        }
    }

    /// Атомарное сохранение.
    pub fn save(&self, root: &Path) -> Result<(), crate::storage::StorageError> {
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&self.map)?;
        crate::storage::repository::atomic_write(&path, &json)
    }

    pub fn get(&self, id: &str) -> Option<&CompatRecord> {
        self.map.get(id)
    }

    pub fn set(&mut self, id: &str, status: CompatStatus, note: String) {
        self.map.insert(
            id.to_string(),
            CompatRecord {
                status,
                note,
                tested_at: {
                    // YYYY-MM-DD без chrono: из SystemTime.
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    days_to_date(secs / 86400)
                },
            },
        );
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Дни от epoch → "YYYY-MM-DD" (григорианский календарь, без зависимостей).
fn days_to_date(days: u64) -> String {
    // Алгоритм civil_from_days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = CompatStore::load(tmp.path());
        assert_eq!(store.len(), 0);
        store.set("general", CompatStatus::Supported, "youtube OK".into());
        store.set(
            "general-alt6",
            CompatStatus::PartiallySupported,
            "discord slow".into(),
        );
        store.save(tmp.path()).unwrap();

        let loaded = CompatStore::load(tmp.path());
        assert_eq!(loaded.len(), 2);
        let rec = loaded.get("general-alt6").unwrap();
        assert_eq!(rec.status, CompatStatus::PartiallySupported);
        assert_eq!(rec.note, "discord slow");
        assert!(!rec.tested_at.is_empty());
    }

    #[test]
    fn corrupted_file_degrades_to_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = CompatStore::path(tmp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        let store = CompatStore::load(tmp.path());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn days_to_date_known_values() {
        assert_eq!(days_to_date(0), "1970-01-01");
        assert_eq!(days_to_date(20454), "2026-01-01"); // 2026-01-01
        assert_eq!(days_to_date(20454 + 315), "2026-11-12");
    }
}
