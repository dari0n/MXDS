//! Экран «Профили»: выбор, статусы совместимости, создание профилей.

use crate::strategy::CompatStatus;

use super::super::app::{CoreCommand, MxdsApp};

/// Цветной бейдж статуса совместимости.
fn compat_badge(ui: &mut egui::Ui, status: CompatStatus) -> egui::Response {
    let (text, color) = match status {
        CompatStatus::Supported => ("✓ работает", egui::Color32::from_rgb(60, 180, 75)),
        CompatStatus::PartiallySupported => ("~ частично", egui::Color32::from_rgb(230, 180, 40)),
        CompatStatus::Unsupported => ("✗ не работает", egui::Color32::LIGHT_RED),
        CompatStatus::NotTested => ("? не тестирован", egui::Color32::GRAY),
    };
    ui.colored_label(color, text)
}

/// Следующий статус при клике на бейдж (цикл).
fn next_status(s: CompatStatus) -> CompatStatus {
    match s {
        CompatStatus::NotTested => CompatStatus::Supported,
        CompatStatus::Supported => CompatStatus::PartiallySupported,
        CompatStatus::PartiallySupported => CompatStatus::Unsupported,
        CompatStatus::Unsupported => CompatStatus::NotTested,
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut MxdsApp) {
    ui.heading("Профили стратегий");
    ui.label("Профиль определяет набор DPI-техник. Смена профиля перезапускает работающий движок.");
    ui.label("Клик по бейджу совместимости отмечает результат проверки (цикл: ? → ✓ → ~ → ✗).");
    if ui.button("Обновить список").clicked() {
        app.send(CoreCommand::RefreshProfiles);
    }
    ui.add_space(8.0);

    let profiles = app.profiles.clone();
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("profiles_grid")
            .num_columns(5)
            .striped(true)
            .show(ui, |ui| {
                ui.strong("");
                ui.strong("Профиль");
                ui.strong("Совместимость");
                ui.strong("Правил");
                ui.strong("Название");
                ui.end_row();
                for p in &profiles {
                    let is_sel = app.active_profile.as_deref() == Some(p.id.as_str());
                    if ui
                        .selectable_label(is_sel, if is_sel { "●" } else { "○" })
                        .clicked()
                        && !is_sel
                    {
                        app.send(CoreCommand::SelectProfile(p.id.clone()));
                    }
                    ui.label(&p.id);
                    let badge = compat_badge(ui, p.compat);
                    if badge.clicked() {
                        app.send(CoreCommand::MarkCompat {
                            id: p.id.clone(),
                            status: next_status(p.compat),
                        });
                    }
                    if let Some(note) = &p.compat_note {
                        if !note.is_empty() {
                            badge.on_hover_text(note);
                        }
                    }
                    ui.label(p.rules_count.to_string());
                    ui.label(&p.name);
                    ui.end_row();
                }
            });

        ui.add_space(12.0);
        ui.separator();
        ui.collapsing("Создать профиль", |ui| {
            ui.label(
                "Новый профиль — копия существующего или пустой шаблон. \
                      Подробнее: docs/PROFILE_GUIDE.md",
            );
            ui.horizontal(|ui| {
                ui.label("Идентификатор:");
                ui.text_edit_singleline(&mut app.new_profile_id);
                ui.label("(латиница, цифры и дефис — имя файла profiles/<id>.json)");
            });
            ui.horizontal(|ui| {
                ui.label("На основе:");
                let current = app.new_profile_from.as_deref().unwrap_or("(пустой шаблон)");
                egui::ComboBox::from_id_salt("new_profile_from")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(app.new_profile_from.is_none(), "(пустой шаблон)")
                            .clicked()
                        {
                            app.new_profile_from = None;
                        }
                        for p in &profiles {
                            if ui
                                .selectable_label(
                                    app.new_profile_from.as_deref() == Some(p.id.as_str()),
                                    &p.id,
                                )
                                .clicked()
                            {
                                app.new_profile_from = Some(p.id.clone());
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Название:");
                ui.text_edit_singleline(&mut app.new_profile_name);
            });
            ui.horizontal(|ui| {
                ui.label("Описание:");
                ui.text_edit_singleline(&mut app.new_profile_desc);
            });
            let can_create = !app.new_profile_id.trim().is_empty();
            if ui
                .add_enabled(can_create, egui::Button::new("Создать"))
                .clicked()
            {
                app.send(CoreCommand::CreateProfile {
                    id: app.new_profile_id.trim().to_string(),
                    from: app.new_profile_from.clone(),
                    name: app.new_profile_name.trim().to_string(),
                    description: app.new_profile_desc.trim().to_string(),
                });
                app.new_profile_id.clear();
                app.new_profile_name.clear();
                app.new_profile_desc.clear();
            }
        });
    });
}
