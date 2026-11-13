//! О программе.

use crate::gui::app::MxdsApp;

pub fn show(ui: &mut egui::Ui, _app: &mut MxdsApp) {
    ui.heading("О программе");
    ui.add_space(8.0);
    ui.label(format!("MXDS {}", env!("CARGO_PKG_VERSION")));
    ui.label("Самостоятельный Windows-клиент обхода DPI на Rust.");
    ui.label("Функциональный перенос Flowseal/zapret-discord-youtube:");
    ui.label("winws.exe и Batch не используются; WinDivert — только как");
    ui.label("системный сетевой компонент.");
    ui.add_space(8.0);
    ui.label("Лицензия: MIT. Бинарники WinDivert: LGPLv3 (см. docs/SECURITY_REVIEW.md).");
}
