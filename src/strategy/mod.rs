//! Strategy Manager — профили стратегий (Цикл 8).
//! См. docs/MODULE_CONTRACTS.md п.6, docs/STRATEGY_COMPATIBILITY.md.
//!
//! Цикл 4: типы ошибок. Цикл 8: парсер winws-аргументов, списки,
//! StrategyManager (discover/validate/resolve).

pub mod lists;
pub mod manager;
pub mod parser;
pub mod profile;
pub mod profile_compat;
pub mod validation;

pub use lists::{HostMatcher, IpMatcher};
pub use manager::StrategyManager;
pub use parser::{extract_winws_args, parse_winws_args, slug};
pub use profile::{
    CompatStatus, DesyncSpec, DesyncTechnique, FoolingKind, Profile, ProfileId, ProfileMeta,
    ProfileRule, Protocol, SplitPos,
};
pub use profile_compat::{CompatRecord, CompatStore};
pub use validation::{IssueSeverity, ValidationIssue};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum StrategyError {
    #[error("профиль не найден: {0}")]
    NotFound(String),
    #[error("ошибка разбора профиля {file}: {detail}")]
    Parse { file: String, detail: String },
    #[error("профиль не прошёл валидацию")]
    ValidationFailed,
    #[error("отсутствует payload {file} (ожидался sha256 {sha256_expected})")]
    PayloadMissing {
        file: String,
        sha256_expected: String,
    },
    #[error("параметр оригинальной стратегии не поддержан импортом: {0}")]
    ImportUnsupported(String),
}
