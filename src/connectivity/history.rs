//! История проверок доступности: append-only JSON-lines в
//! `data/connectivity-history.jsonl`, ротация по размеру
//! (MODULE_CONTRACTS п.9).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::results::ConnectivityResult;

/// Максимальный размер файла истории до ротации (5 МиБ).
pub const HISTORY_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Относительный путь файла истории.
pub const HISTORY_REL_PATH: &str = "data/connectivity-history.jsonl";

pub fn history_path(root: &Path) -> PathBuf {
    root.join(HISTORY_REL_PATH)
}

/// Запись истории: unix-время + один результат этапа.
#[derive(serde::Serialize)]
struct HistoryRecord<'a> {
    ts: u64,
    #[serde(flatten)]
    result: &'a ConnectivityResult,
}

/// Дописать результаты прогона в историю. Если файл превысил
/// HISTORY_MAX_BYTES — ротируется в connectivity-history.1.jsonl
/// (один бэкап, перезаписывается).
pub fn append(path: &Path, results: &[ConnectivityResult]) -> io::Result<()> {
    if results.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > HISTORY_MAX_BYTES {
            let rotated = path.with_file_name("connectivity-history.1.jsonl");
            // Устаревший бэкап может существовать — remove+rename.
            let _ = std::fs::remove_file(&rotated);
            std::fs::rename(path, &rotated)?;
            tracing::info!(path = %rotated.display(), "история connectivity ротирована");
        }
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut out = String::new();
    for r in results {
        let line =
            serde_json::to_string(&HistoryRecord { ts, result: r }).map_err(io::Error::other)?;
        out.push_str(&line);
        out.push('\n');
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(out.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectivity::results::Stage;
    use crate::diagnostics::CheckStatus;

    fn sample(target: &str) -> ConnectivityResult {
        ConnectivityResult {
            target: target.into(),
            stage: Stage::Dns,
            status: CheckStatus::Pass,
            elapsed: std::time::Duration::from_millis(12),
            detail: "ok".into(),
        }
    }

    #[test]
    fn append_creates_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/connectivity-history.jsonl");
        append(&path, &[sample("a"), sample("b")]).unwrap();
        append(&path, &[sample("c")]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 3);
        let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(v["target"], "a");
        assert_eq!(v["stage"], "Dns");
        assert_eq!(v["elapsed_ms"], 12);
        assert!(v["ts"].as_u64().unwrap() > 0);
    }

    #[test]
    fn rotation_at_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        // Файл заведомо больше лимита.
        std::fs::write(&path, "x".repeat((HISTORY_MAX_BYTES + 1) as usize)).unwrap();
        append(&path, &[sample("new")]).unwrap();
        let rotated = dir.path().join("connectivity-history.1.jsonl");
        assert!(rotated.exists());
        assert!(std::fs::metadata(&rotated).unwrap().len() > HISTORY_MAX_BYTES);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn append_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.jsonl");
        append(&path, &[]).unwrap();
        assert!(!path.exists());
    }
}
