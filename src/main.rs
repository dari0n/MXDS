//! Точка входа MXDS.
//!
//! Без аргументов — GUI (eframe, Цикл 13).
//! С аргументами — CLI (clap, Цикл 12). Коды завершения:
//! docs/ERROR_HANDLING.md п.6.

use mxds::application::AppPaths;
use mxds::config::{ConfigManager, ConfigPaths};
use mxds::logging::{self, LogLevel};

#[tokio::main]
async fn main() {
    // Уровень логирования — из конфигурации (best effort; ENV MXDS_*
    // уже учитывается загрузчиком), fallback — Info. Файловые логи:
    // <root>/logs с ротацией по размеру (Цикл 14).
    let paths = AppPaths::resolve();
    let level = ConfigManager::load(ConfigPaths::resolve(&paths.root))
        .map(|m| m.config().log_level)
        .unwrap_or(LogLevel::Info);
    let _log_guard = logging::init(level, Some(&paths.root.join("logs")));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "MXDS запускается");

    // Без аргументов — GUI; иначе — CLI.
    if std::env::args().len() == 1 {
        std::process::exit(mxds::gui::run());
    }
    let code = mxds::cli::commands::run().await;
    std::process::exit(code);
}
