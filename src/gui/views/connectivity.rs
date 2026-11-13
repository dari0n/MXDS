//! Проверка доступности: запуск, результаты по целям и этапам.

use crate::gui::app::{CoreCommand, MxdsApp};

use super::check_mark;

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Проверка доступности");
    ui.add_space(8.0);

    if ui.button("Проверить сейчас").clicked() {
        app.send(CoreCommand::CheckConnectivity);
    }
    ui.label(
        "Каждая цель проверяется каскадом DNS → TCP → TLS → HTTP. \
              Результат зависит от внешних ресурсов и не гарантируется.",
    );
    ui.add_space(8.0);

    let Some(results) = &app.connectivity else {
        ui.label("проверка ещё не выполнялась");
        return;
    };

    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut current = String::new();
        for r in results {
            if r.target != current {
                current = r.target.clone();
                ui.add_space(6.0);
                ui.strong(&current);
            }
            let (mark, color) = check_mark(r.status);
            ui.horizontal(|ui| {
                ui.colored_label(color, mark);
                ui.monospace(format!("{:?}", r.stage));
                ui.label(format!("{} мс", r.elapsed.as_millis()));
                ui.label(&r.detail);
            });
        }
    });
}
