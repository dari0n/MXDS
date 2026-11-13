//! Корневое приложение eframe (Цикл 13).
//!
//! Архитектура: GUI-поток (eframe) ↔ фоновый поток с tokio-рантаймом,
//! владеющий CoreManager и IPC-сервером. Обмен — mpsc-каналы:
//! CoreCommand вниз, CoreEvent наверх. GUI не содержит бизнес-логики —
//! только вызовы Application Core (PROMPT.md Цикл 13).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::application::{
    AppEvent, AppPaths, AppState, BootMode, CoreError, CoreManager, ProfileInfo,
};
use crate::config::Config;
use crate::connectivity::ConnectivityResult;
use crate::diagnostics::{CheckResult, FixId};
use crate::engine::{EngineState, EngineStats};

use super::views::Screen;

/// Команды от GUI к Application Core.
pub enum CoreCommand {
    StartEngine,
    StopEngine,
    SelectProfile(String),
    /// Перечитать список профилей с диска.
    RefreshProfiles,
    /// Создать профиль (копией from или пустым шаблоном).
    CreateProfile {
        id: String,
        from: Option<String>,
        name: String,
        description: String,
    },
    /// Отметить статус совместимости профиля.
    MarkCompat {
        id: String,
        status: crate::strategy::CompatStatus,
    },
    SaveConfig(Box<Config>),
    RunDiagnostics,
    /// Выполнить авто-исправление одной проверки.
    FixDiagnostic(FixId),
    /// Выполнить все доступные авто-исправления.
    FixAllDiagnostics(Vec<FixId>),
    CheckConnectivity,
    RefreshLogs,
    Shutdown,
}

/// События от Core к GUI.
pub enum CoreEvent {
    /// Снимок состояния + статистика (по тику и по событиям).
    State(Box<AppState>, EngineStats),
    Profiles(Vec<ProfileInfo>),
    Config(Box<Config>),
    Diagnostics(Vec<CheckResult>),
    Connectivity(Vec<ConnectivityResult>),
    Logs(String),
    /// Ошибка выполнения команды (показывается баннером).
    Error(String),
    /// Информационное сообщение об успешной операции (зелёный баннер).
    Info(String),
    /// Core не поднялся — GUI показывает фатальный экран.
    Fatal(String),
}

/// Фоновый поток: tokio-рантайм + CoreManager + IPC-сервер.
fn core_thread(mut rx: mpsc::UnboundedReceiver<CoreCommand>, tx: mpsc::UnboundedSender<CoreEvent>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        // Эксклюзивный режим (Цикл 14): single-instance + очистка
        // осиротевшего драйвера WinDivert от прошлого краша.
        let core = match CoreManager::bootstrap_with(AppPaths::resolve(), BootMode::Exclusive).await
        {
            Ok(c) => Arc::new(c),
            Err(CoreError::Mxds(crate::MxdsError::Windows(
                crate::windows::WindowsError::InstanceAlreadyRunning,
            ))) => {
                let _ = tx.send(CoreEvent::Fatal(
                    "Другой экземпляр MXDS уже запущен.\nЗакройте его или используйте CLI: mxds status / mxds stop."
                        .into(),
                ));
                return;
            }
            Err(e) => {
                let _ = tx.send(CoreEvent::Fatal(e.to_string()));
                return;
            }
        };
        // IPC-сервер: CLI (status/stop/run) управляет этим экземпляром (D-009).
        let ipc = crate::cli::ipc_server::spawn(core.clone());
        let mut events = core.subscribe();

        // Начальные данные.
        let _ = tx.send(CoreEvent::State(
            Box::new(core.state().await),
            core.engine_stats().await,
        ));
        let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
        let _ = tx.send(CoreEvent::Config(Box::new(core.config().await)));

        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                cmd = rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        CoreCommand::StartEngine => {
                            // Автоподбор профиля (если включён в настройках):
                            // проверяем активный и, при необходимости, перебираем
                            // остальные, пока не найдём рабочий.
                            let auto = core.config().await.auto_select;
                            if auto {
                                match core.auto_select_profile().await {
                                    Ok(rep) => match &rep.selected {
                                        Some(id) => {
                                            let _ = tx.send(CoreEvent::Info(format!(
                                                "движок запущен, рабочий профиль: {id}"
                                            )));
                                        }
                                        None => {
                                            let _ = tx.send(CoreEvent::Error(format!(
                                                "рабочий профиль не найден (попыток: {}); \
                                                 движок остановлен",
                                                rep.attempts.len()
                                            )));
                                        }
                                    },
                                    Err(e) => {
                                        let _ =
                                            tx.send(CoreEvent::Error(format!("автоподбор: {e}")));
                                    }
                                }
                                let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                                let _ = tx.send(CoreEvent::State(
                                    Box::new(core.state().await),
                                    core.engine_stats().await,
                                ));
                            } else if let Err(e) = core.start_engine().await {
                                let _ = tx.send(CoreEvent::Error(format!("запуск движка: {e}")));
                            }
                        }
                        CoreCommand::StopEngine => {
                            if let Err(e) = core.stop_engine().await {
                                let _ = tx.send(CoreEvent::Error(format!("остановка: {e}")));
                            }
                        }
                        CoreCommand::SelectProfile(id) => {
                            if let Err(e) = core.select_profile(id).await {
                                let _ = tx.send(CoreEvent::Error(format!("профиль: {e}")));
                            }
                            let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                        }
                        CoreCommand::RefreshProfiles => {
                            let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                        }
                        CoreCommand::CreateProfile { id, from, name, description } => {
                            match core.create_profile(&id, from, Some(name), Some(description)).await {
                                Ok(p) => {
                                    let _ = tx.send(CoreEvent::Info(format!("профиль '{}' создан", p.id)));
                                    let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                                }
                                Err(e) => {
                                    let _ = tx.send(CoreEvent::Error(format!("создание профиля: {e}")));
                                }
                            }
                        }
                        CoreCommand::MarkCompat { id, status } => {
                            if let Err(e) = core.mark_profile_compat(&id, status, String::new()).await {
                                let _ = tx.send(CoreEvent::Error(format!("статус профиля: {e}")));
                            }
                            let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                        }
                        CoreCommand::SaveConfig(draft) => {
                            let result = core
                                .update_config(move |c| *c = *draft)
                                .await;
                            match result {
                                Ok(()) => {
                                    let _ = tx.send(CoreEvent::Config(Box::new(core.config().await)));
                                }
                                Err(e) => {
                                    let _ = tx.send(CoreEvent::Error(format!("конфигурация: {e}")));
                                }
                            }
                        }
                        CoreCommand::RunDiagnostics => {
                            let r = core.run_diagnostics().await;
                            let _ = tx.send(CoreEvent::Diagnostics(r));
                        }
                        CoreCommand::FixDiagnostic(fix) => {
                            match core.run_fix(fix).await {
                                Ok(msg) => {
                                    let _ = tx.send(CoreEvent::Info(msg));
                                }
                                Err(e) => {
                                    let _ = tx.send(CoreEvent::Error(format!("исправление: {e}")));
                                }
                            }
                            // Перезапускаем диагностику — статусы обновятся.
                            let r = core.run_diagnostics().await;
                            let _ = tx.send(CoreEvent::Diagnostics(r));
                        }
                        CoreCommand::FixAllDiagnostics(fixes) => {
                            let total = fixes.len();
                            let mut failed: Vec<String> = Vec::new();
                            for fix in fixes {
                                if let Err(e) = core.run_fix(fix).await {
                                    failed.push(format!("{fix:?}: {e}"));
                                }
                            }
                            if failed.is_empty() {
                                let _ = tx.send(CoreEvent::Info(format!(
                                    "выполнены все исправления ({total})"
                                )));
                            } else {
                                let _ = tx.send(CoreEvent::Error(format!(
                                    "исправлено {} из {}; ошибки: {}",
                                    total - failed.len(),
                                    total,
                                    failed.join("; ")
                                )));
                            }
                            let r = core.run_diagnostics().await;
                            let _ = tx.send(CoreEvent::Diagnostics(r));
                        }
                        CoreCommand::CheckConnectivity => {
                            let r = core.check_connectivity(None).await;
                            let _ = tx.send(CoreEvent::Connectivity(r));
                        }
                        CoreCommand::RefreshLogs => {
                            let c = core.clone();
                            let tx2 = tx.clone();
                            tokio::task::spawn_blocking(move || {
                                let log = read_logs(c.paths().root.as_path(), 300);
                                let _ = tx2.send(CoreEvent::Logs(log));
                            });
                        }
                        CoreCommand::Shutdown => break,
                    }
                }
                ev = events.recv() => {
                    match ev {
                        Ok(AppEvent::EngineStarted)
                        | Ok(AppEvent::EngineStopped)
                        | Ok(AppEvent::EngineFailed(_))
                        | Ok(AppEvent::ProfileChanged(_))
                        | Ok(AppEvent::SettingsChanged) => {
                            let _ = tx.send(CoreEvent::State(
                                Box::new(core.state().await),
                                core.engine_stats().await,
                            ));
                        }
                        Ok(AppEvent::ProfilesChanged) => {
                            let _ = tx.send(CoreEvent::Profiles(core.list_profiles().await));
                        }
                        Ok(AppEvent::AutoSelectProgress(msg)) => {
                            let _ = tx.send(CoreEvent::Info(msg));
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "GUI пропустил события (lagged)");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = tick.tick() => {
                    let _ = tx.send(CoreEvent::State(
                        Box::new(core.state().await),
                        core.engine_stats().await,
                    ));
                }
            }
        }
        ipc.shutdown().await;
        let _ = core.shutdown().await;
    });
}

/// Последние N строк журнала (для экрана «Логи»).
fn read_logs(root: &std::path::Path, max_lines: usize) -> String {
    let dir = root.join("logs");
    let mut candidates: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("mxds.log"))
            .collect(),
        Err(_) => return "(журнал пока пуст)".into(),
    };
    if candidates.is_empty() {
        return "(журнал пока пуст)".into();
    }
    candidates.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    let path = candidates.last().unwrap().path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let skip = lines.len().saturating_sub(max_lines);
    lines[skip..].join("\n")
}

/// Путь к маркеру «подсказка про автоподбор уже показана».
fn autoselect_hint_marker() -> std::path::PathBuf {
    AppPaths::resolve()
        .root
        .join("data")
        .join(".autoselect-hint-shown")
}

pub struct MxdsApp {
    pub screen: Screen,
    tx: mpsc::UnboundedSender<CoreCommand>,
    rx: mpsc::UnboundedReceiver<CoreEvent>,

    // Отображаемое состояние (обновляется событиями Core).
    pub engine_state: EngineState,
    pub active_profile: Option<String>,
    pub last_errors: Vec<String>,
    pub stats: EngineStats,
    pub profiles: Vec<ProfileInfo>,
    pub config: Option<Config>,
    pub config_draft: Option<Config>,
    pub diagnostics: Option<Vec<CheckResult>>,
    pub connectivity: Option<Vec<ConnectivityResult>>,
    pub logs: String,
    pub last_error_banner: Option<String>,
    pub last_info_banner: Option<String>,
    pub fatal: Option<String>,
    /// Показать одноразовую подсказку про автоподбор (первый запуск GUI).
    pub show_autoselect_hint: bool,
    pub elevated: bool,
    // Форма создания профиля (экран «Профили»).
    pub new_profile_id: String,
    pub new_profile_from: Option<String>,
    pub new_profile_name: String,
    pub new_profile_desc: String,
}

impl MxdsApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("mxds-core".into())
            .spawn(move || core_thread(cmd_rx, ev_tx))
            .expect("core thread");

        Self {
            screen: Screen::Home,
            tx: cmd_tx,
            rx: ev_rx,
            engine_state: EngineState::Stopped,
            active_profile: None,
            last_errors: Vec::new(),
            stats: EngineStats::default(),
            profiles: Vec::new(),
            config: None,
            config_draft: None,
            diagnostics: None,
            connectivity: None,
            logs: String::new(),
            last_error_banner: None,
            last_info_banner: None,
            fatal: None,
            show_autoselect_hint: false,
            elevated: crate::windows::is_elevated(),
            new_profile_id: String::new(),
            new_profile_from: None,
            new_profile_name: String::new(),
            new_profile_desc: String::new(),
        }
    }

    pub fn send(&self, cmd: CoreCommand) {
        let _ = self.tx.send(cmd);
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.apply_event(ev);
        }
    }

    /// Применение одного события Core к отображаемому состоянию.
    /// Вынесено из drain_events для тестирования (Цикл 15).
    fn apply_event(&mut self, ev: CoreEvent) {
        match ev {
            CoreEvent::State(state, stats) => {
                self.engine_state = state.engine;
                self.active_profile = state.active_profile;
                self.last_errors = state.last_errors;
                self.stats = stats;
            }
            CoreEvent::Profiles(p) => self.profiles = p,
            CoreEvent::Config(c) => {
                // Одноразовая подсказка про автоподбор: показываем при
                // первом запуске GUI, если автоподбор включён.
                if c.auto_select && !autoselect_hint_marker().exists() {
                    self.show_autoselect_hint = true;
                }
                // Черновик синхронизируем, только если его нет
                // (иначе затираем ввод пользователя).
                if self.config_draft.is_none() {
                    self.config_draft = Some(c.as_ref().clone());
                }
                self.config = Some(*c);
            }
            CoreEvent::Diagnostics(d) => self.diagnostics = Some(d),
            CoreEvent::Connectivity(c) => self.connectivity = Some(c),
            CoreEvent::Logs(l) => self.logs = l,
            CoreEvent::Error(e) => self.last_error_banner = Some(e),
            CoreEvent::Info(m) => self.last_info_banner = Some(m),
            CoreEvent::Fatal(e) => self.fatal = Some(e),
        }
    }
}

impl eframe::App for MxdsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("MXDS");
                ui.separator();
                super::views::status_badge(ui, self.engine_state);
                if !self.elevated {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "без прав администратора — движок не запустится",
                    );
                }
                if let Some(p) = &self.active_profile {
                    ui.separator();
                    ui.label(format!("профиль: {p}"));
                }
            });
        });

        egui::SidePanel::left("nav").show(ctx, |ui| {
            ui.heading("Разделы");
            ui.separator();
            for screen in Screen::ALL {
                if ui
                    .selectable_label(self.screen == *screen, screen.title())
                    .clicked()
                {
                    self.screen = *screen;
                    // Автоподгрузка данных при входе на экран.
                    if screen == &Screen::Logs {
                        self.send(CoreCommand::RefreshLogs);
                    }
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(fatal) = &self.fatal {
                ui.colored_label(egui::Color32::RED, format!("Ядро не запущено: {fatal}"));
                return;
            }
            if let Some(err) = self.last_error_banner.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, &err);
                    if ui.button("×").clicked() {
                        self.last_error_banner = None;
                    }
                });
                ui.separator();
            }
            if let Some(info) = self.last_info_banner.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(0x3c, 0xb0, 0x50), &info);
                    if ui.button("×").clicked() {
                        self.last_info_banner = None;
                    }
                });
                ui.separator();
            }
            match self.screen {
                Screen::Home => super::views::home::show(ui, self),
                Screen::Engine => super::views::engine::show(ui, self),
                Screen::Profiles => super::views::profiles::show(ui, self),
                Screen::Config => super::views::config::show(ui, self),
                Screen::Connectivity => super::views::connectivity::show(ui, self),
                Screen::Diagnostics => super::views::diagnostics::show(ui, self),
                Screen::Logs => super::views::logs::show(ui, self),
                Screen::Help => super::views::help::show(ui, self),
                Screen::About => super::views::about::show(ui, self),
            }
        });

        // Одноразовое окно-подсказка про автоподбор профиля.
        if self.show_autoselect_hint {
            let mut open = true;
            egui::Window::new("Автоподбор профиля")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("При запуске движка MXDS сам подбирает рабочий профиль.");
                    ui.add_space(4.0);
                    ui.label(
                        "Обратите внимание: первый перебор может занять пару минут — \
                         прогресс виден в баннере сверху. Дальнейшие запуски будут \
                         быстрыми: рабочий профиль запоминается.",
                    );
                    ui.add_space(4.0);
                    ui.label("Отключить автоподбор можно на экране «Конфигурация».");
                    ui.add_space(8.0);
                    if ui.button("Понятно").clicked() {
                        self.show_autoselect_hint = false;
                    }
                });
            if !open {
                self.show_autoselect_hint = false;
            }
            if !self.show_autoselect_hint {
                // Маркер: больше не показывать.
                let marker = autoselect_hint_marker();
                if let Some(dir) = marker.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&marker, b"shown");
            }
        }

        // Тиковые события (статистика) — перерисовка раз в секунду.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.send(CoreCommand::Shutdown);
        // Дать фоновому потоку время остановить движок и IPC.
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

#[cfg(test)]
impl MxdsApp {
    /// Конструктор для тестов: каналы без фонового core-потока.
    fn new_for_test() -> (Self, mpsc::UnboundedSender<CoreEvent>) {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let app = Self {
            screen: Screen::Home,
            tx: cmd_tx,
            rx: ev_rx,
            engine_state: EngineState::Stopped,
            active_profile: None,
            last_errors: Vec::new(),
            stats: EngineStats::default(),
            profiles: Vec::new(),
            config: None,
            config_draft: None,
            diagnostics: None,
            connectivity: None,
            logs: String::new(),
            last_error_banner: None,
            last_info_banner: None,
            fatal: None,
            show_autoselect_hint: false,
            elevated: false,
            new_profile_id: String::new(),
            new_profile_from: None,
            new_profile_name: String::new(),
            new_profile_desc: String::new(),
        };
        (app, ev_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppState;
    use crate::config::Config;
    use crate::logging::LogLevel;

    #[test]
    fn state_event_updates_displayed_state() {
        let (mut app, _tx) = MxdsApp::new_for_test();
        let mut state = AppState::new();
        state.engine = EngineState::Running;
        state.active_profile = Some("general".into());
        state.push_error("тестовая ошибка".into());
        let stats = EngineStats {
            packets_received: 42,
            ..Default::default()
        };
        app.apply_event(CoreEvent::State(Box::new(state), stats));
        assert_eq!(app.engine_state, EngineState::Running);
        assert_eq!(app.active_profile.as_deref(), Some("general"));
        assert_eq!(app.last_errors, vec!["тестовая ошибка".to_string()]);
        assert_eq!(app.stats.packets_received, 42);
    }

    #[test]
    fn config_event_does_not_overwrite_user_draft() {
        let (mut app, _tx) = MxdsApp::new_for_test();
        let cfg = Config::default();
        app.apply_event(CoreEvent::Config(Box::new(cfg.clone())));
        assert!(app.config.is_some());
        assert!(app.config_draft.is_some());
        // Пользователь отредактировал черновик.
        app.config_draft.as_mut().unwrap().log_level = LogLevel::Trace;
        // Новое событие конфигурации не должно затереть черновик.
        app.apply_event(CoreEvent::Config(Box::new(cfg)));
        assert_eq!(
            app.config_draft.as_ref().unwrap().log_level,
            LogLevel::Trace
        );
    }

    #[test]
    fn error_and_fatal_events_set_banners() {
        let (mut app, _tx) = MxdsApp::new_for_test();
        app.apply_event(CoreEvent::Error("сбой движка".into()));
        assert_eq!(app.last_error_banner.as_deref(), Some("сбой движка"));
        assert!(app.fatal.is_none());
        app.apply_event(CoreEvent::Fatal("ядро не поднялось".into()));
        assert_eq!(app.fatal.as_deref(), Some("ядро не поднялось"));
    }

    #[test]
    fn logs_and_lists_events_replace_content() {
        let (mut app, _tx) = MxdsApp::new_for_test();
        app.apply_event(CoreEvent::Logs("line1\nline2".into()));
        assert_eq!(app.logs, "line1\nline2");
        app.apply_event(CoreEvent::Profiles(vec![]));
        assert!(app.profiles.is_empty());
        app.apply_event(CoreEvent::Diagnostics(vec![]));
        assert_eq!(app.diagnostics.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn read_logs_empty_dir_is_placeholder() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_logs(tmp.path(), 50), "(журнал пока пуст)");
        // Каталог logs создан, но файлов нет.
        std::fs::create_dir_all(tmp.path().join("logs")).unwrap();
        assert_eq!(read_logs(tmp.path(), 50), "(журнал пока пуст)");
    }

    #[test]
    fn read_logs_returns_tail_of_newest_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        // Старый ротированный файл.
        std::fs::write(dir.join("mxds.log.1"), "old\n").unwrap();
        // Активный файл — новее (mtime). Небольшая пауза для различения.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let content: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        std::fs::write(dir.join("mxds.log"), content).unwrap();
        let tail = read_logs(tmp.path(), 3);
        assert_eq!(tail, "line8\nline9\nline10");
    }
}
