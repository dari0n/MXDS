//! Connectivity — проверка доступности ресурсов (Цикл 11).
//! См. docs/MODULE_CONTRACTS.md п.9, PROMPT.md Цикл 11.
//!
//! Каскад DNS→TCP→TLS→HTTP (провал этапа блокирует вышестоящие),
//! ICMP ping, история в data/connectivity-history.jsonl.

pub mod checker;
pub mod history;
pub mod results;

pub use checker::{check, load_targets, parse_targets, DEFAULT_TIMEOUT};
pub use results::{ConnectivityResult, Stage, Target};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConnectivityError {
    #[error("DNS: {0}")]
    Dns(String),
    #[error("TCP: {0}")]
    Tcp(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("HTTP: статус {status}")]
    Http { status: u16 },
    #[error("тайм-аут на этапе {stage:?} после {elapsed_ms} мс")]
    Timeout {
        stage: results::Stage,
        elapsed_ms: u64,
    },
    #[error("нет интернет-соединения")]
    NoInternet,
}
