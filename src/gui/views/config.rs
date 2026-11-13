//! Конфигурация: редактирование через черновик, сохранение — только
//! через CoreManager::update_config (валидация до записи).

use crate::config::GameFilterMode;
use crate::gui::app::{CoreCommand, MxdsApp};
use crate::logging::LogLevel;

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Конфигурация");
    ui.add_space(8.0);

    let Some(draft) = app.config_draft.as_mut() else {
        ui.label("загрузка...");
        return;
    };

    egui::Grid::new("config_grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Уровень логирования:");
            egui::ComboBox::from_id_salt("log_level")
                .selected_text(format!("{:?}", draft.log_level))
                .show_ui(ui, |ui| {
                    for level in [
                        LogLevel::Error,
                        LogLevel::Warn,
                        LogLevel::Info,
                        LogLevel::Debug,
                        LogLevel::Trace,
                    ] {
                        ui.selectable_value(&mut draft.log_level, level, format!("{level:?}"));
                    }
                });
            ui.end_row();

            ui.label("Game Filter:");
            egui::ComboBox::from_id_salt("gf_mode")
                .selected_text(format!("{:?}", draft.game_filter.mode))
                .show_ui(ui, |ui| {
                    for mode in [
                        GameFilterMode::Disabled,
                        GameFilterMode::All,
                        GameFilterMode::Tcp,
                        GameFilterMode::Udp,
                    ] {
                        ui.selectable_value(&mut draft.game_filter.mode, mode, format!("{mode:?}"));
                    }
                });
            ui.end_row();

            if draft.game_filter.mode != GameFilterMode::Disabled {
                ui.label("Game Filter TCP-порты:");
                ui.text_edit_singleline(&mut draft.game_filter.tcp_ports);
                ui.end_row();
                ui.label("Game Filter UDP-порты:");
                ui.text_edit_singleline(&mut draft.game_filter.udp_ports);
                ui.end_row();
            }

            ui.label("Длина очереди движка:");
            ui.add(egui::DragValue::new(&mut draft.engine.queue_length).range(64..=65536));
            ui.end_row();
            ui.label("Время в очереди, мс:");
            ui.add(egui::DragValue::new(&mut draft.engine.queue_time_ms).range(100..=10000));
            ui.end_row();
            ui.label("Размер очереди, КиБ:");
            ui.add(egui::DragValue::new(&mut draft.engine.queue_size_kb).range(64..=8192));
            ui.end_row();

            ui.label("Проверять обновления:");
            ui.checkbox(&mut draft.check_updates, "");
            ui.end_row();

            ui.label("Автоподбор профиля:");
            ui.checkbox(&mut draft.auto_select, "");
            ui.end_row();
        });

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Сохранить").clicked() {
            if let Some(draft) = app.config_draft.clone() {
                app.send(CoreCommand::SaveConfig(Box::new(draft)));
                // После сохранения черновик заменится событием Config.
                app.config_draft = None;
            }
        }
        if ui.button("Сбросить изменения").clicked() {
            app.config_draft = app.config.clone();
        }
    });
    ui.label("Изменение параметров движка/Game Filter перезапускает работающий движок.");
}
