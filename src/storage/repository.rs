//! Атомарная запись файлов: temp + rename (WINDOWS_INTEGRATION п.5).

use std::fs;
use std::path::Path;

use crate::storage::StorageError;

/// Атомарно записывает содержимое в path: запись во временный файл
/// в том же каталоге, затем rename. При ошибке temp-файл удаляется.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), StorageError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| {
            StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "путь без имени файла",
            ))
        })?
        .to_string_lossy();

    let tmp = dir.join(format!(".{file_name}.tmp"));
    let write_result = fs::write(&tmp, contents);
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.json");
        atomic_write(&file, b"{\"a\":1}").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn atomic_write_replaces_existing_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.json");
        atomic_write(&file, b"v1").unwrap();
        atomic_write(&file, b"v2").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "v2");
        assert!(!dir.path().join(".test.json.tmp").exists());
    }
}
