//! Core Manager — единственный владелец бизнес-состояния (PROMPT.md п.6,
//! Цикл 9). GUI и CLI используют только этот публичный API.
//!
//! Владеет: ConfigManager, StrategyManager, EngineManager, шина событий,
//! машина состояния приложения. Все методы идемпотентны к повторному
//! вызову (MODULE_CONTRACTS п.2).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::application::events::{AppEvent, EventBus};
use crate::application::state::{AppState, ComponentId};
use crate::config::{Config, ConfigManager, ConfigPaths};
use crate::engine::{EngineManager, EngineState, EngineStats};
use crate::strategy::{ProfileId, StrategyManager};
use crate::MxdsError;

/// Пути приложения (корень — каталог с config/, profiles/, lists/, assets/).
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    /// Корень по умолчанию: каталог исполняемого файла.
    pub fn default_root() -> Self {
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Self { root }
    }

    /// Разрешение корня для CLI: ENV MXDS_ROOT → каталог exe (если в нём
    /// есть config/default.json — установленная копия) → корень
    /// dev-репозитория (сборка из исходников).
    pub fn resolve() -> Self {
        if let Ok(p) = std::env::var("MXDS_ROOT") {
            return Self {
                root: PathBuf::from(p),
            };
        }
        let exe_root = Self::default_root();
        if exe_root.root.join("config").join("default.json").exists() {
            return exe_root;
        }
        // Dev-сборка: exe в target/debug, ресурсы — в корне репозитория.
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if dev.join("config").join("default.json").exists() {
            return Self { root: dev };
        }
        exe_root
    }
}

/// Режим загрузки Core (Цикл 14 — восстановление после сбоя).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    /// Обычный bootstrap (тесты, вспомогательные вызовы): без захвата
    /// single-instance и без очистки WinDivert.
    Plain,
    /// Эксклюзивный запуск (CLI run, GUI): захват single-instance
    /// мьютекса и очистка «осиротевшего» драйвера WinDivert от
    /// предыдущего аварийного завершения.
    Exclusive,
}

/// Информация о профиле для GUI/CLI (без внутренностей правил).
#[derive(Debug, Clone)]
pub struct ProfileInfo {
    pub id: ProfileId,
    pub name: String,
    pub description: String,
    pub compat: crate::strategy::CompatStatus,
    /// Заметка о проверке совместимости (data/profile-compat.json).
    pub compat_note: Option<String>,
    pub rules_count: usize,
    pub is_active: bool,
}

/// Ошибка Core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("приложение уже остановлено")]
    Shutdown,
    #[error("нет активного профиля")]
    NoActiveProfile,
    #[error("mxds: {0}")]
    Mxds(#[from] MxdsError),
}

/// Итог автоподбора профиля (auto_select_profile).
#[derive(Debug, Clone)]
pub struct AutoSelectReport {
    /// Выбранный рабочий профиль (None — ни один не заработал).
    pub selected: Option<String>,
    /// Попытки по порядку: профиль, успех, деталь проверки.
    pub attempts: Vec<AutoSelectAttempt>,
}

/// Одна попытка автоподбора.
#[derive(Debug, Clone)]
pub struct AutoSelectAttempt {
    pub profile: String,
    pub ok: bool,
    pub detail: String,
}

/// Максимум попыток автоподбора (полный перебор 22 профилей — минуты).
const MAX_AUTO_ATTEMPTS: usize = 8;

/// Приоритет статуса совместимости при автоподборе (меньше — раньше).
fn compat_rank(c: crate::strategy::CompatStatus) -> u8 {
    use crate::strategy::CompatStatus as C;
    match c {
        C::Supported => 0,
        C::NotTested => 1,
        C::PartiallySupported => 2,
        C::Unsupported => 3,
    }
}

/// Порядок кандидатов автоподбора: активный первым (быстрый путь,
/// когда всё уже работает), затем проверенные рабочие → непроверенные →
/// частичные → нерабочие. Сортировка стабильна — внутри группы
/// сохраняется порядок discover().
fn order_candidates(profiles: &[ProfileInfo]) -> Vec<ProfileId> {
    let mut v: Vec<&ProfileInfo> = profiles.iter().collect();
    v.sort_by_key(|p| (!p.is_active, compat_rank(p.compat)));
    v.into_iter().map(|p| p.id.clone()).collect()
}

/// Core Manager.
pub struct CoreManager {
    state: Arc<RwLock<AppState>>,
    bus: EventBus,
    config: RwLock<ConfigManager>,
    strategy: StrategyManager,
    engine: RwLock<EngineManager>,
    paths: AppPaths,
    is_shutdown: RwLock<bool>,
    /// Удерживаемый single-instance захват (только BootMode::Exclusive).
    _instance: Option<crate::windows::SingleInstance>,
}

impl CoreManager {
    /// Bootstrap: загрузка конфигурации, обнаружение профилей,
    /// подготовка менеджеров. Движок НЕ запускается (это команда).
    /// Эквивалентно `bootstrap_with(paths, BootMode::Plain)`.
    pub async fn bootstrap(paths: AppPaths) -> Result<Self, CoreError> {
        Self::bootstrap_with(paths, BootMode::Plain).await
    }

    /// Bootstrap с явным режимом (Цикл 14). В режиме Exclusive:
    /// 1) захват single-instance (другой экземпляр → ошибка);
    /// 2) очистка «осиротевшего» драйвера WinDivert от прошлого краша
    ///    (best effort — ошибка только логируется).
    pub async fn bootstrap_with(paths: AppPaths, mode: BootMode) -> Result<Self, CoreError> {
        let instance = match mode {
            BootMode::Plain => None,
            BootMode::Exclusive => {
                let inst = crate::windows::SingleInstance::acquire().map_err(MxdsError::Windows)?;
                match crate::windows::windivert_cleanup() {
                    Ok(()) => {
                        tracing::info!("восстановление: осиротевший драйвер WinDivert очищен")
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "восстановление: очистка WinDivert не удалась")
                    }
                }
                Some(inst)
            }
        };

        let config_paths = ConfigPaths {
            default_path: paths.root.join("config").join("default.json"),
            config_path: paths.root.join("config").join("config.json"),
        };
        let config = ConfigManager::load(config_paths).map_err(MxdsError::Config)?;
        for w in &config.load_warnings {
            tracing::warn!("config: {w}");
        }
        let strategy = StrategyManager::new(&paths.root);

        let core = Self {
            state: Arc::new(RwLock::new(AppState::new())),
            bus: EventBus::new(),
            config: RwLock::new(config),
            strategy,
            engine: RwLock::new(EngineManager::new()),
            paths,
            is_shutdown: RwLock::new(false),
            _instance: instance,
        };

        // Восстановление состояния: активный профиль из конфигурации.
        let active = core.config.read().await.config().active_profile.clone();
        if let Some(id) = &active {
            core.state.write().await.active_profile = Some(id.clone());
        }
        core.bus.publish(AppEvent::ApplicationStarted);
        tracing::info!(active_profile = ?active, ?mode, "ядро приложения загружено");
        Ok(core)
    }

    /// Снимок состояния (engine state, active profile, last errors).
    pub async fn state(&self) -> AppState {
        // Фактическое состояние движка — источник истины.
        let engine_state = self.engine.read().await.state();
        let mut s = self.state.read().await.clone();
        s.engine = engine_state;
        s
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AppEvent> {
        self.bus.subscribe()
    }

    // --- Движок ---

    /// Запуск движка с активным профилем. Идемпотентно: при RUNNING — Ok(()).
    pub async fn start_engine(&self) -> Result<(), CoreError> {
        self.ensure_running().await?;
        if self.engine.read().await.state().is_running() {
            return Ok(()); // no-op
        }
        let profile_id = self
            .state
            .read()
            .await
            .active_profile
            .clone()
            .ok_or(CoreError::NoActiveProfile)?;

        let profile = self
            .strategy
            .load(&profile_id)
            .map_err(MxdsError::Strategy)?;
        let cfg = self.config.read().await.config().clone();
        let plan = self
            .strategy
            .resolve(&profile, &cfg)
            .map_err(MxdsError::Strategy)?;

        let mut engine = self.engine.write().await;
        match engine.start(&plan, &cfg.engine) {
            Ok(()) => {
                drop(engine);
                self.bus.publish(AppEvent::EngineStarted);
                tracing::info!(profile = %profile_id, "движок запущен");
                Ok(())
            }
            Err(e) => {
                drop(engine);
                let msg = e.to_string();
                self.state.write().await.push_error(msg.clone());
                self.bus.publish(AppEvent::EngineFailed(msg));
                Err(MxdsError::Engine(e).into())
            }
        }
    }

    /// Остановка движка. Идемпотентно: при STOPPED — Ok(()).
    pub async fn stop_engine(&self) -> Result<(), CoreError> {
        let mut engine = self.engine.write().await;
        match engine.state() {
            EngineState::Stopped => Ok(()), // no-op
            _ => {
                engine.stop().map_err(MxdsError::Engine)?;
                drop(engine);
                self.bus.publish(AppEvent::EngineStopped);
                tracing::info!("движок остановлен");
                Ok(())
            }
        }
    }

    /// Статистика движка.
    pub async fn engine_stats(&self) -> EngineStats {
        self.engine.read().await.stats()
    }

    // --- Диагностика ---

    /// Полный прогон диагностики (MODULE_CONTRACTS п.2, п.8).
    /// Результат публикуется событием DiagnosticCompleted.
    pub async fn run_diagnostics(&self) -> Vec<crate::diagnostics::CheckResult> {
        use crate::diagnostics::{default_checks, run_all, DiagContext};

        let config_issues = self.config.read().await.validate();
        let engine_state = self.engine.read().await.state();
        let engine_error = if engine_state == EngineState::Failed {
            self.state.read().await.last_errors.last().cloned()
        } else {
            None
        };
        let ctx = DiagContext {
            root: self.paths.root.clone(),
            dll_path: crate::windivert::adapter::default_dll_path(),
            engine_state,
            engine_error,
            config_issues,
        };
        // Проверки — блокирующие (SCM, реестр, netsh) — выносим из
        // async-рантайма.
        let results = tokio::task::spawn_blocking(move || run_all(&ctx, default_checks()))
            .await
            .unwrap_or_else(|e| {
                vec![crate::diagnostics::CheckResult {
                    name: "diagnostics".into(),
                    status: crate::diagnostics::CheckStatus::Fail,
                    description: "прогон диагностики прерван".into(),
                    duration: std::time::Duration::ZERO,
                    details: Some(e.to_string()),
                    recommendation: None,
                    fix: None,
                }]
            });
        self.bus
            .publish(AppEvent::DiagnosticCompleted(results.clone()));
        let pass = results
            .iter()
            .filter(|r| r.status == crate::diagnostics::CheckStatus::Pass)
            .count();
        let warn = results
            .iter()
            .filter(|r| r.status == crate::diagnostics::CheckStatus::Warn)
            .count();
        let fail = results
            .iter()
            .filter(|r| r.status == crate::diagnostics::CheckStatus::Fail)
            .count();
        tracing::info!(
            total = results.len(),
            pass,
            warn,
            fail,
            "диагностика завершена"
        );
        results
    }

    /// Выполнить авто-исправление диагностики (кнопка «Исправить» в GUI).
    /// Требует прав администратора — проверка внутри diagnostics::run_fix.
    /// Возвращает человекочитаемый итог операции.
    pub async fn run_fix(
        &self,
        fix: crate::diagnostics::FixId,
    ) -> Result<String, crate::MxdsError> {
        let message = tokio::task::spawn_blocking(move || crate::diagnostics::run_fix(fix))
            .await
            .map_err(|e| {
                crate::MxdsError::Windows(crate::windows::WindowsError::Ipc(format!(
                    "задача исправления прервана: {e}"
                )))
            })?
            .map_err(crate::MxdsError::from)?;
        tracing::info!(?fix, message = %message, "авто-исправление выполнено");
        Ok(message)
    }

    // --- Проверка доступности ---

    /// Проверка доступности целей (MODULE_CONTRACTS п.2, п.9).
    /// Без явного списка — цели из config/targets.txt (fallback —
    /// встроенный набор). Результаты дописываются в историю
    /// data/connectivity-history.jsonl и публикуются событием
    /// ConnectivityCheckCompleted.
    pub async fn check_connectivity(
        &self,
        targets: Option<Vec<crate::connectivity::Target>>,
    ) -> Vec<crate::connectivity::ConnectivityResult> {
        let targets =
            targets.unwrap_or_else(|| crate::connectivity::load_targets(&self.paths.root));
        let results =
            crate::connectivity::check(&targets, crate::connectivity::DEFAULT_TIMEOUT).await;
        let history = crate::connectivity::history::history_path(&self.paths.root);
        if let Err(e) = crate::connectivity::history::append(&history, &results) {
            tracing::warn!(error = %e, "история connectivity не записана");
        }
        self.bus
            .publish(AppEvent::ConnectivityCheckCompleted(results.clone()));
        let ok = results
            .iter()
            .filter(|r| r.status == crate::diagnostics::CheckStatus::Pass)
            .count();
        tracing::info!(
            targets = results.len(),
            ok,
            failed = results.len() - ok,
            "проверка доступности завершена"
        );
        results
    }

    // --- Профили ---

    /// Список профилей с флагом активного.
    pub async fn list_profiles(&self) -> Vec<ProfileInfo> {
        let active = self.state.read().await.active_profile.clone();
        self.strategy
            .discover()
            .unwrap_or_default()
            .into_iter()
            .map(|p| ProfileInfo {
                is_active: active.as_deref() == Some(p.id.as_str()),
                compat_note: self.strategy.compat_note(&p.id),
                id: p.id.clone(),
                name: p.meta.name.clone(),
                description: p.meta.description.clone(),
                compat: p.compat,
                rules_count: p.rules.len(),
            })
            .collect()
    }

    /// Подробности одного профиля (None — не найден/невалиден).
    pub async fn profile_info(&self, id: &str) -> Option<ProfileInfo> {
        self.list_profiles().await.into_iter().find(|p| p.id == id)
    }

    /// Создать профиль (копией существующего или пустым шаблоном).
    /// Публикует ProfilesChanged. Работающий движок не затрагивается,
    /// пока новый профиль не выбран.
    pub async fn create_profile(
        &self,
        id: &str,
        from: Option<String>,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<ProfileInfo, CoreError> {
        self.ensure_running().await?;
        self.strategy
            .create(id, from.as_deref(), name, description)
            .map_err(MxdsError::Strategy)?;
        self.bus.publish(AppEvent::ProfilesChanged);
        tracing::info!(profile = %id, "профиль создан через Core");
        self.profile_info(id).await.ok_or_else(|| {
            MxdsError::Strategy(crate::strategy::StrategyError::NotFound(id.into())).into()
        })
    }

    /// Сохранить результат проверки совместимости профиля
    /// (например, после перебора scripts/profile-sweep.ps1).
    /// Публикует ProfilesChanged.
    pub async fn mark_profile_compat(
        &self,
        id: &str,
        status: crate::strategy::CompatStatus,
        note: String,
    ) -> Result<(), CoreError> {
        self.ensure_running().await?;
        self.strategy
            .mark_compat(id, status, note)
            .map_err(MxdsError::Strategy)?;
        self.bus.publish(AppEvent::ProfilesChanged);
        Ok(())
    }

    /// Заметка о проверке совместимости (для отображения).
    pub async fn compat_note(&self, id: &str) -> Option<String> {
        self.strategy.compat_note(id)
    }

    /// Валидация текущей конфигурации.
    pub async fn validate_config(&self) -> Vec<crate::strategy::validation::ValidationIssue> {
        self.config.read().await.validate()
    }

    /// Выбор профиля: валидация, сохранение в конфигурацию, событие.
    /// При работающем движке — перезапуск с новым профилем.
    pub async fn select_profile(&self, id: ProfileId) -> Result<(), CoreError> {
        self.ensure_running().await?;
        // Профиль должен существовать и валидироваться без ошибок.
        let profile = self.strategy.load(&id).map_err(MxdsError::Strategy)?;
        let errors: Vec<_> = self
            .strategy
            .validate(&profile)
            .into_iter()
            .filter(|i| i.severity == crate::strategy::IssueSeverity::Error)
            .collect();
        if !errors.is_empty() {
            return Err(
                MxdsError::Strategy(crate::strategy::StrategyError::ValidationFailed).into(),
            );
        }

        let was_running = self.engine.read().await.state().is_running();
        if was_running {
            self.stop_engine().await?;
        }

        // Сохраняем выбор в конфигурацию.
        self.config
            .write()
            .await
            .update(|c| c.active_profile = Some(id.clone()))
            .map_err(MxdsError::Config)?;
        self.state.write().await.active_profile = Some(id.clone());
        self.bus.publish(AppEvent::ProfileChanged(id.clone()));
        tracing::info!(profile = %id, "профиль выбран");

        if was_running {
            self.start_engine().await?;
        }
        Ok(())
    }

    /// Автоподбор рабочего профиля при запуске (post-release).
    ///
    /// Последовательно запускает движок с профилями-кандидатами и
    /// проверяет YouTube каскадом доступности. Первый профиль с успешным
    /// HTTP-этапом становится активным, движок остаётся запущенным на нём.
    /// Результаты каждой попытки отмечаются в data/profile-compat.json,
    /// прогресс публикуется событиями AppEvent::AutoSelectProgress.
    ///
    /// Если ни один кандидат не заработал — движок останавливается,
    /// выбор профиля откатывается к исходному, report.selected == None.
    pub async fn auto_select_profile(&self) -> Result<AutoSelectReport, CoreError> {
        use crate::connectivity::Stage;
        use crate::diagnostics::CheckStatus;
        use crate::strategy::CompatStatus;
        use std::time::Duration;

        self.ensure_running().await?;
        let profiles = self.list_profiles().await;
        if profiles.is_empty() {
            return Err(CoreError::NoActiveProfile);
        }
        let original = self.state.read().await.active_profile.clone();
        let candidates = order_candidates(&profiles);
        let total = candidates.len().min(MAX_AUTO_ATTEMPTS);
        let mut attempts: Vec<AutoSelectAttempt> = Vec::new();
        let mut selected: Option<String> = None;

        for (i, id) in candidates.into_iter().take(total).enumerate() {
            self.bus.publish(AppEvent::AutoSelectProgress(format!(
                "автоподбор: проверяю профиль {id} ({}/{total})",
                i + 1
            )));

            let attempt = async {
                self.select_profile(id.clone()).await?;
                self.start_engine().await?;
                // Ждём RUNNING (открытие драйвера занимает до пары секунд).
                let mut waited = Duration::ZERO;
                while !self.engine.read().await.state().is_running()
                    && self.engine.read().await.state() != EngineState::Failed
                    && waited < Duration::from_secs(10)
                {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    waited += Duration::from_millis(250);
                }
                if !self.engine.read().await.state().is_running() {
                    return Err(CoreError::Mxds(MxdsError::Windows(
                        crate::windows::WindowsError::Ipc(
                            "движок не перешёл в RUNNING за 10 с".into(),
                        ),
                    )));
                }
                // Небольшая пауза: дать фильтрам примениться.
                tokio::time::sleep(Duration::from_millis(800)).await;
                Ok::<(), CoreError>(())
            }
            .await;

            let (ok, detail) = match attempt {
                Err(e) => (false, e.to_string()),
                Ok(()) => {
                    let results = self
                        .check_connectivity(Some(vec![crate::connectivity::Target::Url {
                            name: "YouTubeWeb".into(),
                            url: "https://www.youtube.com/".into(),
                        }]))
                        .await;
                    match results.iter().find(|r| r.stage == Stage::Http) {
                        Some(r) if r.status == CheckStatus::Pass => {
                            (true, format!("youtube OK за {} мс", r.elapsed.as_millis()))
                        }
                        Some(r) => (false, format!("http: {}", r.detail)),
                        None => {
                            let last = results
                                .last()
                                .map(|r| format!("{:?}: {}", r.stage, r.detail))
                                .unwrap_or_default();
                            (false, format!("каскад не дошёл до HTTP ({last})"))
                        }
                    }
                }
            };

            self.bus
                .publish(AppEvent::AutoSelectProgress(format!("{id}: {detail}")));
            let status = if ok {
                CompatStatus::Supported
            } else {
                CompatStatus::Unsupported
            };
            let _ = self
                .mark_profile_compat(&id, status, format!("автоподбор: {detail}"))
                .await;
            attempts.push(AutoSelectAttempt {
                profile: id.clone(),
                ok,
                detail,
            });
            if ok {
                selected = Some(id);
                break;
            }
        }

        if selected.is_none() {
            // Ни один кандидат не заработал — останавливаем движок
            // и возвращаем исходный профиль.
            let _ = self.stop_engine().await;
            if let Some(orig) = original {
                let _ = self.select_profile(orig).await;
            }
        }
        tracing::info!(
            selected = ?selected,
            tried = attempts.len(),
            "автоподбор профиля завершён"
        );
        Ok(AutoSelectReport { selected, attempts })
    }

    // --- Конфигурация ---

    pub async fn config(&self) -> Config {
        self.config.read().await.config().clone()
    }

    /// Контролируемое изменение конфигурации (валидация до записи).
    /// Если меняются параметры, влияющие на движок (queue/game filter),
    /// и движок работает — он перезапускается.
    pub async fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<(), CoreError> {
        self.ensure_running().await?;
        let before = self.config.read().await.config().clone();
        let issues = {
            let mut cfg = self.config.write().await;
            cfg.update(f).map_err(MxdsError::Config)?
        };
        if issues
            .iter()
            .any(|i| i.severity == crate::strategy::IssueSeverity::Error)
        {
            return Err(
                MxdsError::Config(crate::config::ConfigError::Validation(issues.len())).into(),
            );
        }
        let after = self.config.read().await.config().clone();
        // Синхронизируем активный профиль в состоянии с конфигурацией.
        if after.active_profile != before.active_profile {
            self.state.write().await.active_profile = after.active_profile.clone();
        }
        let needs_restart = after.engine.queue_length != before.engine.queue_length
            || after.engine.queue_time_ms != before.engine.queue_time_ms
            || after.engine.queue_size_kb != before.engine.queue_size_kb
            || after.game_filter.mode != before.game_filter.mode
            || after.game_filter.tcp_ports != before.game_filter.tcp_ports
            || after.game_filter.udp_ports != before.game_filter.udp_ports;
        self.bus.publish(AppEvent::SettingsChanged);

        if needs_restart && self.engine.read().await.state().is_running() {
            tracing::info!("параметры движка изменены — перезапуск");
            self.stop_engine().await?;
            self.start_engine().await?;
        }
        Ok(())
    }

    // --- Жизненный цикл ---

    /// Корректная остановка: движок → событие → состояние Shutdown.
    /// Повторный вызов — no-op.
    pub async fn shutdown(&self) -> Result<(), CoreError> {
        {
            let mut flag = self.is_shutdown.write().await;
            if *flag {
                return Ok(());
            }
            *flag = true;
        }
        let _ = self.stop_engine().await; // уже остановлен — не ошибка
        self.bus.publish(AppEvent::ApplicationStopped);
        self.bus.publish(AppEvent::ComponentStateChanged(
            ComponentId::Engine,
            "stopped".into(),
        ));
        tracing::info!("ядро приложения остановлено");
        Ok(())
    }

    async fn ensure_running(&self) -> Result<(), CoreError> {
        if *self.is_shutdown.read().await {
            Err(CoreError::Shutdown)
        } else {
            Ok(())
        }
    }

    /// Пути приложения (для модулей, которым нужен root).
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> AppPaths {
        // Изолированная копия корня: тесты пишут config.json, параллельный
        // прогон на общем корне недопустим (гонка temp+rename).
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let copy = |rel: &str| {
            let from = manifest.join(rel);
            let to = root.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(&from, &to).unwrap();
        };
        copy("config/default.json");
        fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let dest = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    std::fs::create_dir_all(&dest).unwrap();
                    copy_dir(&entry.path(), &dest);
                } else {
                    std::fs::create_dir_all(to).unwrap();
                    std::fs::copy(entry.path(), &dest).unwrap();
                }
            }
        }
        for dir in ["profiles", "lists", "assets"] {
            copy_dir(&manifest.join(dir), &root.join(dir));
        }
        // TempDir должен жить весь тест; проще — забыть (тестовый процесс
        // завершится, мусор во TEMP ничтожен).
        std::mem::forget(tmp);
        AppPaths { root }
    }

    #[tokio::test]
    async fn bootstrap_loads_and_publishes_started() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let mut rx = core.subscribe();
        // ApplicationStarted уже опубликовано до subscribe — проверяем,
        // что state доступен и профили обнаружены.
        assert!(!core.list_profiles().await.is_empty());
        core.shutdown().await.unwrap();
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AppEvent::ApplicationStopped));
    }

    #[tokio::test]
    async fn start_without_profile_is_error() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        // В репозитории config.json может не быть; гарантируем None.
        core.update_config(|c| c.active_profile = None)
            .await
            .unwrap();
        let r = core.start_engine().await;
        assert!(matches!(r, Err(CoreError::NoActiveProfile)));
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn select_profile_persists_and_events() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let mut rx = core.subscribe();
        core.select_profile("general".into()).await.unwrap();
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AppEvent::ProfileChanged(id) if id == "general"));
        assert_eq!(
            core.config().await.active_profile.as_deref(),
            Some("general")
        );
        // состояние
        assert_eq!(
            core.state().await.active_profile.as_deref(),
            Some("general")
        );
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn select_unknown_profile_fails() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let r = core.select_profile("no-such-profile".into()).await;
        assert!(r.is_err());
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stop_without_start_is_noop() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        assert!(core.stop_engine().await.is_ok());
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn double_shutdown_is_noop() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        core.shutdown().await.unwrap();
        core.shutdown().await.unwrap();
        // после shutdown команды отклоняются
        let r = core.select_profile("general".into()).await;
        assert!(matches!(r, Err(CoreError::Shutdown)));
    }

    #[tokio::test]
    async fn update_config_rejects_invalid() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let before = core.config().await;
        // невалидные порты Game Filter — должно отклониться
        let r = core
            .update_config(|c| c.game_filter.tcp_ports = "99999".into())
            .await;
        assert!(r.is_err());
        // конфигурация не изменилась
        assert_eq!(
            core.config().await.game_filter.tcp_ports,
            before.game_filter.tcp_ports
        );
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn run_diagnostics_returns_results_and_event() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let mut rx = core.subscribe();
        let results = core.run_diagnostics().await;
        assert!(results.len() >= 15);
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, AppEvent::DiagnosticCompleted(_)));
        // На dev-машине файлы WinDivert валидны.
        let wf = results
            .iter()
            .find(|r| r.name == "windivert_files")
            .unwrap();
        assert_eq!(
            wf.status,
            crate::diagnostics::CheckStatus::Pass,
            "{:?}",
            wf.details
        );
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn profile_listing_marks_active() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        core.select_profile("general-alt".into()).await.unwrap();
        let profiles = core.list_profiles().await;
        assert_eq!(profiles.len(), 22);
        let active: Vec<_> = profiles.iter().filter(|p| p.is_active).collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "general-alt");
        assert_eq!(active[0].rules_count, 9);
        core.shutdown().await.unwrap();
        // cleanup: вернуть активный профиль в None, чтобы тесты не
        // зависели от порядка (config.json — рабочий файл разработки)
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        core.update_config(|c| c.active_profile = None)
            .await
            .unwrap();
        core.shutdown().await.unwrap();
    }

    // --- Цикл 15: восстановление после сбоя (BootMode) ---

    // allow: токio-рантайм теста однопоточный, удержание std-MutexGuard
    // через await здесь — сознательная сериализация доступа к глобальному
    // мьютексу single-instance; дедлок исключён (захват короткий,
    // конкурентный тест просто ждёт на lock()).
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn bootstrap_exclusive_fails_while_instance_held() {
        // Глобальный мьютекс один на процесс — сериализуемся с тестом
        // SingleInstance в windows::components.
        let _serialization = crate::windows::components::tests::SI_TEST_LOCK
            .lock()
            .unwrap();
        let held = crate::windows::SingleInstance::acquire().unwrap();
        let r = CoreManager::bootstrap_with(test_paths(), BootMode::Exclusive).await;
        assert!(
            matches!(
                r,
                Err(CoreError::Mxds(MxdsError::Windows(
                    crate::windows::WindowsError::InstanceAlreadyRunning,
                )))
            ),
            "ожидался InstanceAlreadyRunning, получено: {:?}",
            r.err()
        );
        drop(held);
    }

    #[allow(clippy::await_holding_lock)] // см. обоснование выше
    #[tokio::test]
    async fn bootstrap_plain_ignores_held_instance() {
        let _serialization = crate::windows::components::tests::SI_TEST_LOCK
            .lock()
            .unwrap();
        let held = crate::windows::SingleInstance::acquire().unwrap();
        // Plain не трогает single-instance и не очищает WinDivert.
        let core = CoreManager::bootstrap_with(test_paths(), BootMode::Plain)
            .await
            .unwrap();
        core.shutdown().await.unwrap();
        drop(held);
    }

    // --- Управление профилями: создание и статусы совместимости ---

    #[tokio::test]
    async fn create_profile_template_mark_and_list() {
        let paths = test_paths();
        let core = CoreManager::bootstrap(paths.clone()).await.unwrap();
        let mut rx = core.subscribe();

        // Создание пустым шаблоном.
        let info = core
            .create_profile("test-custom", None, Some("Тестовый".into()), None)
            .await
            .unwrap();
        assert_eq!(info.id, "test-custom");
        assert_eq!(info.name, "Тестовый");
        assert_eq!(info.compat, crate::strategy::CompatStatus::NotTested);
        assert!(matches!(
            rx.recv().await.unwrap(),
            AppEvent::ProfilesChanged
        ));

        // Отметка совместимости сохраняется и видна в списке.
        core.mark_profile_compat(
            "test-custom",
            crate::strategy::CompatStatus::Supported,
            "youtube OK".into(),
        )
        .await
        .unwrap();
        let info = core.profile_info("test-custom").await.unwrap();
        assert_eq!(info.compat, crate::strategy::CompatStatus::Supported);
        assert_eq!(info.compat_note.as_deref(), Some("youtube OK"));

        // Файл существует и валиден при повторной загрузке (тот же корень).
        let again = CoreManager::bootstrap(paths).await.unwrap();
        let info2 = again.profile_info("test-custom").await.unwrap();
        assert_eq!(info2.compat, crate::strategy::CompatStatus::Supported);
        core.shutdown().await.unwrap();
        again.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn create_profile_from_base_copies_rules() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        let base = core.profile_info("general-alt6").await.unwrap();
        let info = core
            .create_profile("test-copy", Some("general-alt6".into()), None, None)
            .await
            .unwrap();
        assert_eq!(info.rules_count, base.rules_count);
        core.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn create_profile_rejects_duplicates_and_bad_slugs() {
        let core = CoreManager::bootstrap(test_paths()).await.unwrap();
        // Существующий id.
        assert!(core
            .create_profile("general", None, None, None)
            .await
            .is_err());
        // Плохие slug.
        for bad in ["", "-x", "UPPER", "с русским", "с пробелом", "a/b"] {
            assert!(
                core.create_profile(bad, None, None, None).await.is_err(),
                "slug '{bad}' должен отклоняться"
            );
        }
        // mark несуществующего профиля — ошибка.
        assert!(core
            .mark_profile_compat(
                "no-such",
                crate::strategy::CompatStatus::Supported,
                String::new()
            )
            .await
            .is_err());
        core.shutdown().await.unwrap();
    }

    #[test]
    fn auto_select_candidate_ordering() {
        use crate::strategy::CompatStatus as C;
        let mk = |id: &str, compat: C, is_active: bool| ProfileInfo {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            compat,
            compat_note: None,
            rules_count: 0,
            is_active,
        };
        let profiles = vec![
            mk("a-unsupported", C::Unsupported, false),
            mk("b-active", C::Unsupported, true),
            mk("c-supported", C::Supported, false),
            mk("d-fresh", C::NotTested, false),
            mk("e-partial", C::PartiallySupported, false),
        ];
        let order = order_candidates(&profiles);
        assert_eq!(
            order,
            vec![
                "b-active",
                "c-supported",
                "d-fresh",
                "e-partial",
                "a-unsupported"
            ],
            "активный первым, затем по статусу совместимости"
        );
    }
}
