//! Главная: состояние приложения, движка, WinDivert, доступность
//! Discord/YouTube, последние ошибки (PROMPT.md Цикл 13).

use crate::diagnostics::CheckStatus;
use crate::engine::EngineState;
use crate::gui::app::{CoreCommand, MxdsApp};

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Главная");
    ui.add_space(8.0);

    // Крупное управление.
    ui.horizontal(|ui| {
        match app.engine_state {
            EngineState::Stopped | EngineState::Failed => {
                if ui
                    .button(egui::RichText::new("▶  Запустить").size(18.0))
                    .clicked()
                {
                    app.send(CoreCommand::StartEngine);
                }
            }
            EngineState::Running => {
                if ui
                    .button(egui::RichText::new("■  Остановить").size(18.0))
                    .clicked()
                {
                    app.send(CoreCommand::StopEngine);
                }
            }
            EngineState::Starting | EngineState::Stopping => {
                ui.add_enabled(false, egui::Button::new("..."));
            }
        }
        super::status_badge(ui, app.engine_state);
    });
    ui.add_space(12.0);

    egui::Grid::new("home_grid").num_columns(2).show(ui, |ui| {
        ui.label("Состояние приложения:");
        ui.label("работает");
        ui.end_row();

        ui.label("Сетевой компонент:");
        ui.label(match app.engine_state {
            EngineState::Running => "движок активен",
            EngineState::Failed => "ошибка движка",
            _ => "движок остановлен",
        });
        ui.end_row();

        ui.label("Активный профиль:");
        ui.label(app.active_profile.as_deref().unwrap_or("не выбран"));
        ui.end_row();

        ui.label("WinDivert:");
        let wd = app
            .diagnostics
            .as_ref()
            .and_then(|d| d.iter().find(|r| r.name == "windivert_driver"))
            .map(|r| r.description.as_str())
            .unwrap_or("ещё не проверялся (см. раздел «Диагностика»)");
        ui.label(wd);
        ui.end_row();

        for (title, target) in [("Discord", "DiscordMain"), ("YouTube", "YouTubeWeb")] {
            ui.label(format!("Доступность {title}:"));
            match target_status(app, target) {
                Some((CheckStatus::Pass, d)) => {
                    ui.colored_label(egui::Color32::from_rgb(0x3c, 0xb0, 0x50), d)
                }
                Some((CheckStatus::Fail, d)) => ui.colored_label(egui::Color32::RED, d),
                Some((_, d)) => ui.colored_label(egui::Color32::GOLD, d),
                None => ui.label("ещё не проверялась (см. раздел «Доступность»)"),
            };
            ui.end_row();
        }
    });

    ui.add_space(12.0);
    ui.separator();
    ui.heading("Последние ошибки");
    if app.last_errors.is_empty() {
        ui.label("ошибок нет");
    } else {
        for e in app.last_errors.iter().rev().take(5) {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        }
    }
}

/// HTTP-этап последней проверки доступности по имени цели.
fn target_status(app: &MxdsApp, target: &str) -> Option<(CheckStatus, String)> {
    let results = app.connectivity.as_ref()?;
    let r = results
        .iter()
        .filter(|r| r.target == target)
        .find(|r| matches!(r.stage, crate::connectivity::Stage::Http))
        .or_else(|| results.iter().find(|r| r.target == target))?;
    Some((r.status, format!("{:?}: {}", r.stage, r.detail)))
}
