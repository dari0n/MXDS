//! Файловый писатель с ротацией по размеру (Цикл 14).
//! Активный файл — `<dir>/mxds.log`; при превышении лимита он
//! переименовывается в mxds.log.1, прежние .N сдвигаются,
//! старше `keep` — удаляются.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Максимальный размер одного файла журнала (10 МиБ).
pub const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Сколько ротированных файлов хранить (mxds.log.1 … mxds.log.N).
pub const KEEP_LOG_FILES: u32 = 5;

/// Имя активного файла журнала.
pub const LOG_FILE_NAME: &str = "mxds.log";

struct Inner {
    file: File,
    size: u64,
}

/// Писатель, реализующий `io::Write` + `MakeWriter` (клонируемый
/// через Arc). Потокобезопасен; запись короткая, блокировка
/// неконкурентная при штатном объёме логов.
#[derive(Clone)]
pub struct SizeRotatingWriter {
    dir: PathBuf,
    max_bytes: u64,
    keep: u32,
    inner: Arc<Mutex<Inner>>,
}

impl SizeRotatingWriter {
    pub fn new(dir: PathBuf, max_bytes: u64, keep: u32) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(LOG_FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            dir,
            max_bytes,
            keep,
            inner: Arc::new(Mutex::new(Inner { file, size })),
        })
    }

    /// Ротация: mxds.log.(N-1) → .N, …, mxds.log → .1, новый mxds.log.
    fn rotate(&self, inner: &mut Inner) -> io::Result<()> {
        // Закрываем текущий файл перед переименованием (Windows).
        // SAFETY-уровень: заменяем на пустышку, реальный файл откроем ниже.
        inner.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.dir.join(LOG_FILE_NAME).with_extension("rot-tmp"))?;
        let current = self.dir.join(LOG_FILE_NAME);
        // Удалить старейший.
        let oldest = self.dir.join(format!("{LOG_FILE_NAME}.{}", self.keep));
        let _ = std::fs::remove_file(&oldest);
        for i in (1..self.keep).rev() {
            let from = self.dir.join(format!("{LOG_FILE_NAME}.{i}"));
            if from.exists() {
                let to = self.dir.join(format!("{LOG_FILE_NAME}.{}", i + 1));
                let _ = std::fs::rename(&from, &to);
            }
        }
        let first = self.dir.join(format!("{LOG_FILE_NAME}.1"));
        let _ = std::fs::rename(&current, &first);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)?;
        // Удалить пустышку.
        let _ = std::fs::remove_file(self.dir.join(LOG_FILE_NAME).with_extension("rot-tmp"));
        inner.file = file;
        inner.size = 0;
        Ok(())
    }
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inner.size + buf.len() as u64 > self.max_bytes {
            self.rotate(&mut inner)?;
        }
        let n = inner.file.write(buf)?;
        inner.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SizeRotatingWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::fmt::MakeWriter;

    #[test]
    fn rotates_at_size_limit_and_keeps_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = SizeRotatingWriter::new(dir.path().to_path_buf(), 1000, 3).unwrap();
        // 10 записей по 400 байт → несколько ротаций.
        for i in 0..10 {
            let line = format!("{:0396}\n", i);
            w.write_all(line.as_bytes()).unwrap();
        }
        w.flush().unwrap();
        let current = std::fs::read_to_string(dir.path().join(LOG_FILE_NAME)).unwrap();
        assert!(!current.is_empty());
        // Ротированных файлов не больше keep.
        let rotated: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("mxds.log."))
            .collect();
        assert!(rotated.len() <= 3, "ротировано: {rotated:?}");
        // Самое старое содержимое вытеснено (запись 0 не должна
        // пережить все ротации при таком лимите).
        let all: String = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect();
        assert!(all.contains(&format!("{:0396}", 9)));
        assert!(!all.contains(&format!("{:0396}", 0)));
        // Регрессия: служебный файл-пустышка ротации не должен
        // оставаться в каталоге журналов.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("rot-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "остатки ротации: {leftovers:?}");
    }

    #[test]
    fn make_writer_returns_working_clone() {
        let dir = tempfile::tempdir().unwrap();
        let w = SizeRotatingWriter::new(dir.path().to_path_buf(), 10_000, 2).unwrap();
        let mut w2 = w.make_writer();
        w2.write_all(b"hello\n").unwrap();
        let content = std::fs::read_to_string(dir.path().join(LOG_FILE_NAME)).unwrap();
        assert_eq!(content, "hello\n");
    }
}
