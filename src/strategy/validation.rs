//! Результат валидации профиля/конфигурации.

/// Найденная проблема валидации (не фатальная для загрузки списка
/// профилей в целом: невалидный профиль помечается, а не роняет остальные).
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
    pub severity: IssueSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Error,
    Warning,
}
