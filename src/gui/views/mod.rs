//! Представления (экраны) GUI (Цикл 13).

pub mod about;
pub mod config;
pub mod connectivity;
pub mod diagnostics;
pub mod engine;
pub mod help;
pub mod home;
pub mod logs;
pub mod profiles;

use crate::engine::EngineState;

/// Экраны приложения (боковая навигация).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Engine,
    Profiles,
    Config,
    Connectivity,
    Diagnostics,
    Logs,
    Help,
    About,
}

impl Screen {
    pub const ALL: &'static [Screen] = &[
        Screen::Home,
        Screen::Engine,
        Screen::Profiles,
        Screen::Config,
        Screen::Connectivity,
        Screen::Diagnostics,
        Screen::Logs,
        Screen::Help,
        Screen::About,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Home => "Главная",
            Screen::Engine => "Движок",
            Screen::Profiles => "Профили",
            Screen::Config => "Конфигурация",
            Screen::Connectivity => "Доступность",
            Screen::Diagnostics => "Диагностика",
            Screen::Logs => "Логи",
            Screen::Help => "Помощь",
            Screen::About => "О программе",
        }
    }
}

/// Цветной индикатор состояния движка (STOPPED/STARTING/RUNNING/
/// STOPPING/FAILED — PROMPT.md Цикл 13).
pub fn status_badge(ui: &mut egui::Ui, state: EngineState) {
    let (text, color) = match state {
        EngineState::Running => ("RUNNING", egui::Color32::from_rgb(0x3c, 0xb0, 0x50)),
        EngineState::Stopped => ("STOPPED", egui::Color32::GRAY),
        EngineState::Starting => ("STARTING", egui::Color32::GOLD),
        EngineState::Stopping => ("STOPPING", egui::Color32::GOLD),
        EngineState::Failed => ("FAILED", egui::Color32::RED),
    };
    ui.colored_label(color, format!("■ {text}"));
}

/// Маркер статуса проверки — общий для экранов «Диагностика»
/// и «Доступность».
pub fn check_mark(s: crate::diagnostics::CheckStatus) -> (&'static str, egui::Color32) {
    use crate::diagnostics::CheckStatus;
    match s {
        CheckStatus::Pass => ("[OK]", egui::Color32::from_rgb(0x3c, 0xb0, 0x50)),
        CheckStatus::Warn => ("[!]", egui::Color32::GOLD),
        CheckStatus::Fail => ("[X]", egui::Color32::RED),
        CheckStatus::Blocked => ("[-]", egui::Color32::GRAY),
    }
}
