//! Менеджер движка: жизненный цикл, рабочий цикл пакетов, статистика.
//! См. docs/MODULE_CONTRACTS.md п.3, docs/WINDIVERT_INTEGRATION.md п.5.
//!
//! Движок не читает конфиг и не знает о GUI/CLI: вход — EnginePlan
//! (уже разрешённый профиль: строка фильтра, режим desync, payload'ы).
//! Преобразование ResolvedProfile → EnginePlan — Strategy Manager (Цикл 8).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::engine::EngineConfig;
use super::errors::EngineError;
use super::lifecycle::EngineState;
use super::packet::PacketInfo;
use super::rule::{self, FlowTable};
use crate::windivert::{DivertHandle, DivertParam};

/// Вход движка: разрешённый план обработки трафика.
/// Все пути payload'ов прочитаны наверху, списки скомпилированы в
/// матчеры (Цикл 8, Strategy Manager). Пустой список правил —
/// passthrough (движок поднят, трафик не модифицируется).
#[derive(Clone, Default)]
pub struct EnginePlan {
    /// Строка фильтра WinDivert (сгенерирована filter::build_filter
    /// по wf_tcp/wf_udp профиля + GameFilter).
    pub filter: String,
    /// Упорядоченные правила; первое совпавшее выигрывает.
    pub rules: Vec<rule::EngineRule>,
}

impl std::fmt::Debug for EnginePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnginePlan")
            .field("filter", &self.filter)
            .field("rules", &self.rules)
            .finish()
    }
}

/// Хендл запущенного движка: статистика + остановка.
pub struct EngineHandle {
    stats: Arc<StatsInner>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<JoinHandle<()>>,
    state: Arc<Mutex<EngineState>>,
}

#[derive(Default)]
struct StatsInner {
    packets_received: AtomicU64,
    packets_sent: AtomicU64,
    packets_dropped: AtomicU64,
    fakes_sent: AtomicU64,
}

pub struct EngineManager {
    handle: Option<EngineHandle>,
}

impl Default for EngineManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineManager {
    pub fn new() -> Self {
        Self { handle: None }
    }

    pub fn state(&self) -> EngineState {
        match &self.handle {
            None => EngineState::Stopped,
            Some(h) => *h.state.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    pub fn stats(&self) -> super::engine::EngineStats {
        match &self.handle {
            None => super::engine::EngineStats::default(),
            Some(h) => super::engine::EngineStats {
                packets_received: h.stats.packets_received.load(Ordering::Relaxed),
                packets_sent: h.stats.packets_sent.load(Ordering::Relaxed),
                packets_dropped: h.stats.packets_dropped.load(Ordering::Relaxed),
                fakes_sent: h.stats.fakes_sent.load(Ordering::Relaxed),
            },
        }
    }

    /// Запуск: открыть хендл, настроить очередь, поднять рабочий поток.
    /// Идемпотентность: повторный start при живом движке — AlreadyRunning.
    pub fn start(&mut self, plan: &EnginePlan, cfg: &EngineConfig) -> Result<(), EngineError> {
        if self.state().is_running() {
            return Err(EngineError::AlreadyRunning);
        }
        if plan.filter.is_empty() {
            return Err(EngineError::InvalidProfile(
                "пустой фильтр WinDivert".into(),
            ));
        }
        self.stop_internal(); // добить предыдущий Failed/Stopping

        // Прежде чем поднимать движок: убрать осиротевший драйвер
        // (WINDOWS_INTEGRATION п.7). Требует elevation — уже проверена
        // в DivertHandle::open ниже; cleanup делаем best-effort.
        let _ = crate::windows::components::windivert_cleanup();

        let divert = DivertHandle::open(&plan.filter)?;
        // Проверка совместимости драйвера 2.x (WINDIVERT_INTEGRATION п.9).
        let (major, minor) = divert.driver_version()?;
        if major != 2 {
            return Err(crate::windivert::DivertError::VersionMismatch {
                expected_major: 2,
                got_major: major,
                got_minor: minor,
            }
            .into());
        }
        tracing::info!(major, minor, "WinDivert драйвер загружен");

        // Параметры очереди из конфигурации (валидированы Config Manager).
        divert.set_param(DivertParam::QueueLength(cfg.queue_length as u64))?;
        divert.set_param(DivertParam::QueueTimeMs(cfg.queue_time_ms as u64))?;
        divert.set_param(DivertParam::QueueSizeBytes(cfg.queue_size_kb as u64 * 1024))?;

        let stats = Arc::new(StatsInner::default());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let state = Arc::new(Mutex::new(EngineState::Running));
        let plan = plan.clone();

        let worker = {
            let stats = Arc::clone(&stats);
            let stop = Arc::clone(&stop);
            let state = Arc::clone(&state);
            std::thread::Builder::new()
                .name("mxds-engine".into())
                .spawn(move || run_loop(divert, plan, stats, stop, state))
                .map_err(|e| EngineError::WorkerCrashed(e.to_string()))?
        };

        self.handle = Some(EngineHandle {
            stats,
            stop,
            worker: Some(worker),
            state,
        });
        tracing::info!("движок запущен");
        Ok(())
    }

    /// Graceful stop: Shutdown(RECV) → поток выходит из recv → join.
    /// Drop DivertHandle (Close) — при разрушении потока.
    pub fn stop(&mut self) -> Result<(), EngineError> {
        match self.state() {
            EngineState::Stopped => Err(EngineError::NotRunning),
            _ => {
                self.stop_internal();
                Ok(())
            }
        }
    }

    fn stop_internal(&mut self) {
        if let Some(mut h) = self.handle.take() {
            h.stop.store(true, Ordering::Relaxed);
            // WinDivertShutdown зовётся из потока через проверку stop-флага
            // после recv; здесь — только сигнал и ожидание.
            if let Some(w) = h.worker.take() {
                let _ = w.join();
            }
            *h.state.lock().unwrap_or_else(|e| e.into_inner()) = EngineState::Stopped;
        }
    }

    pub fn handle(&self) -> Option<&EngineHandle> {
        self.handle.as_ref()
    }
}

impl EngineHandle {
    pub fn state(&self) -> EngineState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Рабочий цикл (выделенный OS-поток): Recv → обработка → Send.
/// Выход: stop-флаг после каждого recv (recv блокирующий — shutdown
/// вызывается тем же потоком при обнаружении флага, это допустимо:
/// WinDivertShutdown потокобезопасна относительно Recv).
fn run_loop(
    divert: DivertHandle,
    plan: EnginePlan,
    stats: Arc<StatsInner>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    state: Arc<Mutex<EngineState>>,
) {
    let mut flows = FlowTable::default();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = divert.shutdown();
            break;
        }
        let pkt = match divert.recv() {
            Ok(p) => p,
            Err(e) => {
                if stop.load(Ordering::Relaxed) {
                    break; // shutdown в процессе — нормальный выход
                }
                tracing::error!(?e, "WinDivertRecv: ошибка, движок остановлен");
                *state.lock().unwrap_or_else(|e| e.into_inner()) = EngineState::Failed;
                return;
            }
        };
        stats.packets_received.fetch_add(1, Ordering::Relaxed);

        // Свои инъекции (Impostor) и loopback не обрабатываем —
        // реинъектируем как есть (WINDIVERT_INTEGRATION п.7).
        if pkt.address.impostor() || pkt.address.loopback() {
            match divert.send(&pkt.data, &pkt.address) {
                Ok(_) => {
                    stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.packets_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            continue;
        }

        let Some(info) = PacketInfo::parse(&pkt.data) else {
            // Не IP — отправляем как есть.
            match divert.send(&pkt.data, &pkt.address) {
                Ok(_) => {
                    stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.packets_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            continue;
        };

        let kind = rule::classify(&info, &pkt.data);

        // Первое совпавшее правило выигрывает (семантика winws).
        let rule = plan
            .rules
            .iter()
            .find(|r| r.matches(&info, &pkt.data, kind));

        let Some(rule) = rule else {
            // passthrough
            match divert.send(&pkt.data, &pkt.address) {
                Ok(_) => {
                    stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.packets_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            continue;
        };

        // cutoff: после N обработанных пакетов соединения — passthrough.
        if !flows.allow(&info, rule.action.cutoff) {
            match divert.send(&pkt.data, &pkt.address) {
                Ok(_) => {
                    stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    stats.packets_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            continue;
        }

        let outgoing = rule::apply(rule, &pkt.data, &info, kind);
        for mut out in outgoing {
            let mut addr = pkt.address;
            if out.fix_checksums {
                divert.calc_checksums(&mut out.data, &mut addr);
            }
            match divert.send(&out.data, &addr) {
                Ok(_) => {
                    stats.packets_sent.fetch_add(1, Ordering::Relaxed);
                    if out.is_fake {
                        stats.fakes_sent.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    stats.packets_dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(?e, "WinDivertSend: пакет потерян");
                }
            }
        }
    }
    *state.lock().unwrap_or_else(|e| e.into_inner()) = EngineState::Stopped;
    tracing::info!(
        received = stats.packets_received.load(Ordering::Relaxed),
        sent = stats.packets_sent.load(Ordering::Relaxed),
        dropped = stats.packets_dropped.load(Ordering::Relaxed),
        "рабочий цикл движка завершён"
    );
}

// Гарантия безопасного завершения: Drop менеджера останавливает движок.
impl Drop for EngineManager {
    fn drop(&mut self) {
        self.stop_internal();
    }
}

/// Блокирующее ожидание остановки с таймаутом (для graceful shutdown
/// приложения; поток завершается по shutdown+stop-флагу за миллисекунды).
pub fn wait_stopped(mgr: &EngineManager, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if mgr.state() == EngineState::Stopped {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    mgr.state() == EngineState::Stopped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_initial_state_stopped() {
        let mgr = EngineManager::new();
        assert_eq!(mgr.state(), EngineState::Stopped);
        assert_eq!(mgr.stats().packets_received, 0);
    }

    #[test]
    fn stop_without_start_is_error() {
        let mut mgr = EngineManager::new();
        assert!(matches!(mgr.stop(), Err(EngineError::NotRunning)));
    }

    #[test]
    fn start_with_empty_filter_rejected() {
        let mut mgr = EngineManager::new();
        let plan = EnginePlan::default();
        let r = mgr.start(&plan, &EngineConfig::default());
        assert!(matches!(r, Err(EngineError::InvalidProfile(_))));
    }
}
