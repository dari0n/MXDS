//! Внутренняя шина событий. См. PROMPT.md раздел 7.
//! Не зависит от GUI и CLI; корректно завершается вместе с приложением.

use tokio::sync::broadcast;

use crate::application::state::ComponentId;
use crate::connectivity::ConnectivityResult;
use crate::diagnostics::CheckResult;

/// События приложения (PROMPT.md, раздел 7).
#[derive(Debug, Clone)]
pub enum AppEvent {
    ApplicationStarted,
    ApplicationStopped,
    EngineStarted,
    EngineStopped,
    EngineFailed(String),
    ProfileChanged(String),
    /// Состав профилей изменился (создание) или обновился статус
    /// совместимости — подписчикам следует перезапросить список.
    ProfilesChanged,
    SettingsChanged,
    ComponentStateChanged(ComponentId, String),
    DiagnosticCompleted(Vec<CheckResult>),
    ConnectivityCheckCompleted(Vec<ConnectivityResult>),
    /// Прогресс автоподбора профиля (попытка N из M, результат проверки).
    AutoSelectProgress(String),
}

/// Broadcast-шина. Переполнение медленным подписчиком — пропуск событий
/// (RecvError::Lagged) с WARN на стороне подписчика; см. ARCHITECTURE п.4.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub const CAPACITY: usize = 256;

    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(Self::CAPACITY);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    /// Публикация best-effort: отсутствие подписчиков — не ошибка.
    pub fn publish(&self, event: AppEvent) {
        let _ = self.sender.send(event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_reaches_subscriber() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.publish(AppEvent::EngineStarted);
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AppEvent::EngineStarted));
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_ok() {
        let bus = EventBus::new();
        bus.publish(AppEvent::ApplicationStopped);
    }
}
