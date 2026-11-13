//! Машина состояний движка (PROMPT.md Цикл 13).

/// Состояния движка. Переходы валидируются — невозможный переход
/// это ошибка программиста, а не пользователя (см. ERROR_HANDLING п.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EngineState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

impl EngineState {
    /// Разрешённые переходы.
    pub fn can_transition_to(self, next: EngineState) -> bool {
        use EngineState::*;
        matches!(
            (self, next),
            (Stopped, Starting)
                | (Starting, Running)
                | (Starting, Failed)
                | (Starting, Stopping)
                | (Running, Stopping)
                | (Running, Failed)
                | (Stopping, Stopped)
                | (Stopping, Failed)
                | (Failed, Stopped)
        )
    }

    pub fn is_running(self) -> bool {
        matches!(self, EngineState::Running)
    }
}

impl std::fmt::Display for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EngineState::Stopped => "STOPPED",
            EngineState::Starting => "STARTING",
            EngineState::Running => "RUNNING",
            EngineState::Stopping => "STOPPING",
            EngineState::Failed => "FAILED",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        use EngineState::*;
        assert!(Stopped.can_transition_to(Starting));
        assert!(Starting.can_transition_to(Running));
        assert!(Running.can_transition_to(Stopping));
        assert!(Stopping.can_transition_to(Stopped));
        assert!(Failed.can_transition_to(Stopped));
    }

    #[test]
    fn invalid_transitions() {
        use EngineState::*;
        assert!(!Stopped.can_transition_to(Running));
        assert!(!Running.can_transition_to(Starting));
        assert!(!Stopped.can_transition_to(Stopping));
    }
}
