//! Конфигурация и статистика движка.

/// Параметры очереди WinDivert и рабочего цикла.
/// Диапазоны валидируются Config Manager (Цикл 5) по пределам
/// из docs/WINDIVERT_INTEGRATION.md п.4.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineConfig {
    /// Максимальная длина очереди пакетов (WinDivert QUEUE_LENGTH).
    pub queue_length: u32,
    /// Минимальное время нахождения пакета в очереди, мс (QUEUE_TIME).
    pub queue_time_ms: u32,
    /// Максимальный размер очереди в байтах (QUEUE_SIZE).
    pub queue_size_kb: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            queue_length: 4096,
            queue_time_ms: 2000,
            queue_size_kb: 8192,
        }
    }
}

/// Счётчики рабочего цикла движка.
#[derive(Debug, Clone, Copy, Default)]
pub struct EngineStats {
    pub packets_received: u64,
    pub packets_sent: u64,
    pub packets_dropped: u64,
    pub fakes_sent: u64,
}
