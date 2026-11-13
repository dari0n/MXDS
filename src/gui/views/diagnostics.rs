//! Диагностика: запуск прогона, результаты со статусами,
//! рекомендациями и кнопками авто-исправления.

use crate::diagnostics::{CheckStatus, FixId};
use crate::gui::app::{CoreCommand, MxdsApp};

use super::check_mark;

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Диагностика");
    ui.add_space(8.0);

    // Собираем доступные исправления заранее (до заимствования results).
    let fixes: Vec<FixId> = app
        .diagnostics
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|r| r.status != CheckStatus::Pass)
        .filter_map(|r| r.fix)
        .collect();

    ui.horizontal(|ui| {
        if ui.button("Запустить диагностику").clicked() {
            app.send(CoreCommand::RunDiagnostics);
        }
        if !fixes.is_empty() {
            let label = format!("Исправить все ({})", fixes.len());
            let btn = ui.add_enabled(app.elevated, egui::Button::new(label));
            if btn.clicked() {
                app.send(CoreCommand::FixAllDiagnostics(fixes.clone()));
            }
        }
        if !app.elevated && !fixes.is_empty() {
            ui.colored_label(
                egui::Color32::YELLOW,
                "для исправлений нужны права администратора",
            );
        }
    });
    ui.add_space(8.0);

    let Some(results) = &app.diagnostics else {
        ui.label("диагностика ещё не выполнялась");
        return;
    };

    let fails = results
        .iter()
        .filter(|r| r.status == CheckStatus::Fail)
        .count();
    if fails > 0 {
        ui.colored_label(egui::Color32::RED, format!("FAIL: {fails} проверок"));
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(0x3c, 0xb0, 0x50),
            "критических проблем не найдено",
        );
    }
    ui.add_space(6.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        for r in results {
            let (mark, color) = check_mark(r.status);
            ui.horizontal(|ui| {
                ui.colored_label(color, mark);
                ui.monospace(&r.name);
                ui.label(&r.description);
                if r.status != CheckStatus::Pass {
                    if let Some(fix) = r.fix {
                        let btn = ui.add_enabled(app.elevated, egui::Button::new("Исправить"));
                        if btn.clicked() {
                            app.send(CoreCommand::FixDiagnostic(fix));
                        }
                    }
                }
            });
            if let Some(d) = &r.details {
                ui.label(format!("    {d}"));
            }
            if let Some(rec) = &r.recommendation {
                ui.colored_label(egui::Color32::LIGHT_BLUE, format!("    → {rec}"));
            }
        }
    });
}
