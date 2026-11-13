//! Настройка логирования на tracing.
//! См. PROMPT.md Цикл 14 и docs/SECURITY_REVIEW.md п.5:
//! содержимое сетевых пакетов не логируется никогда.

mod guard;
mod rotator;

pub use guard::LogGuard;
pub use rotator::{SizeRotatingWriter, KEEP_LOG_FILES, LOG_FILE_NAME, MAX_LOG_FILE_BYTES};

use tracing_subscriber::EnvFilter;

/// Уровень логирования (управляется конфигурацией и ENV MXDS_LOG_LEVEL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_filter(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// Инициализирует глобальный subscriber: fmt в stderr и (если задан
/// каталог) в файл с ротацией по размеру (Цикл 14: активный mxds.log,
/// архивы mxds.log.1…N, общий объём ограничен). Возвращает guard — его
/// нужно удерживать до конца работы процесса, иначе файловые логи
/// потеряются. При ошибке открытия файла — деградация до stderr-only.
pub fn init(level: LogLevel, log_dir: Option<&std::path::Path>) -> LogGuard {
    let filter = EnvFilter::new(level.as_filter());
    let mut guard = LogGuard::default();

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    use tracing_subscriber::prelude::*;
    let file_layer = log_dir.and_then(|dir| {
        match SizeRotatingWriter::new(dir.to_path_buf(), MAX_LOG_FILE_BYTES, KEEP_LOG_FILES) {
            Ok(writer) => {
                let (writer, file_guard) = tracing_appender::non_blocking(writer);
                guard.file_guard = Some(file_guard);
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_ansi(false),
                )
            }
            Err(e) => {
                eprintln!(
                    "MXDS: не удалось открыть журнал в {}: {e}; только stderr",
                    dir.display()
                );
                None
            }
        }
    });

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer);
    tracing::subscriber::set_global_default(registry).expect("logging::init вызывается один раз");

    tracing::info!(level = level.as_filter(), "logging initialized");
    guard
}
