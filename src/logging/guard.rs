/// Guard, удерживающий фоновый писатель файловых логов.
/// При drop корректно сбрасывает буферы.
#[derive(Default)]
pub struct LogGuard {
    pub(crate) file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl std::fmt::Debug for LogGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogGuard")
            .field("file_logging", &self.file_guard.is_some())
            .finish()
    }
}
