//! Application Core — центральный владелец бизнес-состояния.
//! См. docs/ARCHITECTURE.md п.2-3, docs/MODULE_CONTRACTS.md п.2.
//!
//! Полный Core Manager (bootstrap всех компонентов) реализуется
//! в Цикле 9. Здесь — базовые типы состояния и событий.

pub mod events;
pub mod manager;
pub mod state;

pub use events::{AppEvent, EventBus};
pub use manager::{AppPaths, BootMode, CoreError, CoreManager, ProfileInfo};
pub use state::{AppState, ComponentId};
