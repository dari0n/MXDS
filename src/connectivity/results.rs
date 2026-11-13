//! Типы целей и результатов проверки доступности.

use std::time::Duration;

use crate::diagnostics::CheckStatus;

/// Этап каскадной проверки. Успех этапа НЕ доказывает успех вышестоящего
/// (PROMPT.md Цикл 11: DNS ≠ доступность сайта, TCP ≠ рабочий HTTP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Stage {
    Dns,
    Tcp,
    Tls,
    Http,
    Ping,
}

/// Цель проверки.
#[derive(Debug, Clone)]
pub enum Target {
    /// HTTP/HTTPS-ресурс (каскад DNS→TCP→TLS→HTTP).
    Url { name: String, url: String },
    /// Только ping (ICMP) по адресу.
    Ping {
        name: String,
        addr: std::net::IpAddr,
    },
}

/// Результат проверки одного этапа одной цели.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectivityResult {
    pub target: String,
    pub stage: Stage,
    pub status: CheckStatus,
    #[serde(rename = "elapsed_ms")]
    #[serde(with = "elapsed_ms")]
    pub elapsed: Duration,
    pub detail: String,
}

mod elapsed_ms {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}
