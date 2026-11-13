//! Состояние приложения. См. PROMPT.md Цикл 13 (машина состояний движка).

pub use crate::engine::lifecycle::EngineState;

/// Идентификатор компонента для событий ComponentStateChanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ComponentId {
    Engine,
    WinDivert,
    Config,
    Strategy,
    Diagnostics,
    Connectivity,
}

/// Снимок состояния приложения для GUI/CLI.
#[derive(Debug, Clone)]
pub struct AppState {
    pub engine: EngineState,
    pub active_profile: Option<String>,
    /// Последние ошибки (новые в конце), ограничено по размеру.
    pub last_errors: Vec<String>,
}

impl AppState {
    pub const MAX_LAST_ERRORS: usize = 20;

    pub fn new() -> Self {
        Self {
            engine: EngineState::Stopped,
            active_profile: None,
            last_errors: Vec::new(),
        }
    }

    pub fn push_error(&mut self, msg: String) {
        if self.last_errors.len() >= Self::MAX_LAST_ERRORS {
            self.last_errors.remove(0);
        }
        self.last_errors.push(msg);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_error_keeps_bounded() {
        let mut s = AppState::new();
        for i in 0..30 {
            s.push_error(format!("err {i}"));
        }
        assert_eq!(s.last_errors.len(), AppState::MAX_LAST_ERRORS);
        assert_eq!(s.last_errors.last().unwrap(), "err 29");
    }
}
