//! Движок: состояние, статистика, управление.

use crate::engine::EngineState;
use crate::gui::app::{CoreCommand, MxdsApp};

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Управление движком");
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("Состояние:");
        super::status_badge(ui, app.engine_state);
    });
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        let can_start = matches!(app.engine_state, EngineState::Stopped | EngineState::Failed);
        let can_stop = matches!(app.engine_state, EngineState::Running);
        if ui
            .add_enabled(can_start, egui::Button::new("Запустить"))
            .clicked()
        {
            app.send(CoreCommand::StartEngine);
        }
        if ui
            .add_enabled(can_stop, egui::Button::new("Остановить"))
            .clicked()
        {
            app.send(CoreCommand::StopEngine);
        }
    });

    ui.add_space(12.0);
    ui.separator();
    ui.heading("Статистика");
    egui::Grid::new("stats_grid").num_columns(2).show(ui, |ui| {
        ui.label("Пакетов получено:");
        ui.label(app.stats.packets_received.to_string());
        ui.end_row();
        ui.label("Пакетов отправлено:");
        ui.label(app.stats.packets_sent.to_string());
        ui.end_row();
        ui.label("Пакетов отброшено:");
        ui.label(app.stats.packets_dropped.to_string());
        ui.end_row();
        ui.label("Fake-пакетов отправлено:");
        ui.label(app.stats.fakes_sent.to_string());
        ui.end_row();
    });

    if !app.last_errors.is_empty() {
        ui.add_space(12.0);
        ui.separator();
        ui.heading("Ошибки движка");
        for e in app.last_errors.iter().rev().take(10) {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
    }
}
