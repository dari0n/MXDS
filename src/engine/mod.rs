//! Rust Network Engine — собственный сетевой движок (Цикл 7).
//!
//! Цикл 4: типы конфигурации, статистики и жизненного цикла.
//! Цикл 7: рабочий цикл пакетов поверх windivert::adapter,
//! DPI-desync примитивы, генератор фильтров WinDivert.

// engine::engine — структура задана PROMPT.md (раздел 5), осознанно.
pub mod desync;
#[allow(clippy::module_inception)]
pub mod engine;
pub mod errors;
pub mod filter;
pub mod lifecycle;
pub mod manager;
pub mod packet;
pub mod rule;
pub mod sni;

pub use desync::{DesyncMode, Fooling, Outgoing};
pub use engine::{EngineConfig, EngineStats};
pub use errors::EngineError;
pub use filter::{build_filter, parse_port_ranges};
pub use lifecycle::EngineState;
pub use manager::{wait_stopped, EngineHandle, EngineManager, EnginePlan};
pub use packet::{IpAddr, L4Proto, PacketInfo};
pub use rule::{EngineRule, RuleAction, RuleSplitPos, RuleTechnique, TrafficKind};
