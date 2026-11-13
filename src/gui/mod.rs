//! GUI (egui/eframe) — Цикл 13.
//! Экраны: Главная, Движок, Профили, Конфигурация, Доступность,
//! Диагностика, Логи, О программе (PROMPT.md Цикл 13).
//! GUI не содержит бизнес-логики — только Application Core.

pub mod app;
pub mod views;

/// Запуск GUI. Возвращает код завершения процесса.
pub fn run() -> i32 {
    // panic-hook: запись в лог + попытка graceful shutdown
    // (ERROR_HANDLING п.7). В release профиле panic=abort — Цикл 14.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "паника в GUI-потоке");
        default_hook(info);
    }));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MXDS")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([800.0, 520.0]),
        ..Default::default()
    };
    match eframe::run_native(
        "MXDS",
        options,
        Box::new(|cc| Ok(Box::new(app::MxdsApp::new(cc)))),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("mxds: GUI: {e}");
            1
        }
    }
}
