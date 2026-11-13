//! Логи: хвост журнала приложения (read-only).

use crate::gui::app::{CoreCommand, MxdsApp};

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Логи");
    ui.add_space(8.0);

    if ui.button("Обновить").clicked() {
        app.send(CoreCommand::RefreshLogs);
    }
    ui.add_space(6.0);

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let mut text = if app.logs.is_empty() {
                "(журнал пока пуст)"
            } else {
                &app.logs
            };
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
}
