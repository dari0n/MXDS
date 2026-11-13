//! Команды CLI (clap derive) и обработчики. Цикл 12.
//! Бизнес-логики здесь нет: только Application Core напрямую
//! (автономный режим) или через IPC к работающему экземпляру (D-009).
//! Коды завершения: docs/ERROR_HANDLING.md п.6 —
//! 0 ok, 1 ошибка выполнения, 2 аргументы (clap), 3 диагностика FAIL,
//! 4 нет прав.

use clap::{Parser, Subcommand};

use crate::application::{AppPaths, BootMode, CoreError, CoreManager};

use super::ipc_client;
use super::protocol::IpcCommand;

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_DIAG_FAILED: i32 = 3;
pub const EXIT_NO_RIGHTS: i32 = 4;

#[derive(Parser)]
#[command(
    name = "mxds",
    version,
    about = "MXDS — обход DPI для Discord/YouTube (Windows)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Состояние приложения и движка.
    Status,
    /// Проверка доступности целей (DNS→TCP→TLS→HTTP).
    Check,
    /// Системная диагностика (код 3 при наличии FAIL).
    Diagnostics {
        /// Применить доступные авто-исправления и повторить диагностику
        /// (требуются права администратора).
        #[arg(long)]
        fix: bool,
    },
    /// Конфигурация.
    Config {
        #[command(subcommand)]
        cmd: ConfigCommands,
    },
    /// Профили стратегий.
    Profiles {
        #[command(subcommand)]
        cmd: ProfilesCommands,
    },
    /// Запустить обход (требует прав администратора).
    Run {
        /// Профиль; без аргумента — активный из конфигурации.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Остановить работающий экземпляр.
    Stop,
    /// Версия приложения.
    Version,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Показать текущую конфигурацию (JSON).
    Show,
    /// Проверить конфигурацию (код 1 при ошибках валидации).
    Validate,
}

#[derive(Subcommand)]
pub enum ProfilesCommands {
    /// Список профилей.
    List,
    /// Подробности профиля.
    Info {
        /// Идентификатор профиля (см. profiles list).
        id: String,
    },
    /// Создать профиль: копией существующего (--from) или пустым
    /// шаблоном. См. docs/PROFILE_GUIDE.md.
    Create {
        /// Идентификатор (slug: [a-z0-9-], имя файла profiles/<id>.json).
        id: String,
        /// Базовый профиль для копирования (например general-alt6).
        #[arg(long)]
        from: Option<String>,
        /// Отображаемое название.
        #[arg(long)]
        name: Option<String>,
        /// Описание.
        #[arg(long)]
        description: Option<String>,
    },
    /// Отметить результат проверки совместимости (после перебора
    /// scripts/profile-sweep.ps1). Хранится в data/profile-compat.json.
    Mark {
        /// Идентификатор профиля.
        id: String,
        /// Статус: ok | partial | fail | reset.
        status: String,
        /// Заметка, например "youtube OK 0.7s, discord RST".
        #[arg(long)]
        note: Option<String>,
    },
}

/// Точка входа CLI. Возвращает код завершения процесса.
pub async fn run() -> i32 {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        // Без аргументов — GUI (обычно не достигается: main
        // направляет пустые аргументы сразу в gui::run).
        return crate::gui::run();
    };
    dispatch(command).await
}

async fn dispatch(command: Commands) -> i32 {
    match command {
        Commands::Version => {
            println!("mxds {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Commands::Status => cmd_status().await,
        Commands::Check => cmd_check().await,
        Commands::Diagnostics { fix } => cmd_diagnostics(fix).await,
        Commands::Config { cmd } => cmd_config(cmd).await,
        Commands::Profiles { cmd } => cmd_profiles(cmd).await,
        Commands::Run { profile } => cmd_run(profile).await,
        Commands::Stop => cmd_stop().await,
    }
}

async fn bootstrap_core() -> Result<std::sync::Arc<CoreManager>, i32> {
    match CoreManager::bootstrap(AppPaths::resolve()).await {
        Ok(c) => Ok(std::sync::Arc::new(c)),
        Err(e) => {
            eprintln!("mxds: ошибка инициализации: {e}");
            Err(EXIT_ERROR)
        }
    }
}

/// Признак «уже запущен другой экземпляр» в цепочке ошибок bootstrap.
fn is_already_running(e: &CoreError) -> bool {
    matches!(
        e,
        CoreError::Mxds(crate::MxdsError::Windows(
            crate::windows::WindowsError::InstanceAlreadyRunning,
        ))
    )
}

/// Парсинг статуса совместимости из CLI: ok | partial | fail | reset.
fn parse_compat_status(s: &str) -> Option<crate::strategy::CompatStatus> {
    use crate::strategy::CompatStatus as C;
    match s.to_ascii_lowercase().as_str() {
        "ok" | "supported" => Some(C::Supported),
        "partial" | "partially" => Some(C::PartiallySupported),
        "fail" | "unsupported" => Some(C::Unsupported),
        "reset" | "not-tested" | "nottested" => Some(C::NotTested),
        _ => None,
    }
}

/// Символ статуса совместимости для списков.
fn compat_mark(s: crate::strategy::CompatStatus) -> &'static str {
    use crate::strategy::CompatStatus as C;
    match s {
        C::Supported => "✓",
        C::PartiallySupported => "~",
        C::Unsupported => "✗",
        C::NotTested => "?",
    }
}

/// Синхронный IPC-вызов из async-кода — через spawn_blocking,
/// чтобы не блокировать поток рантайма.
async fn ipc_send(cmd: IpcCommand) -> std::io::Result<Option<super::protocol::IpcResponse>> {
    tokio::task::spawn_blocking(move || ipc_client::try_send(&cmd))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
}

async fn cmd_status() -> i32 {
    // Сначала — работающий экземпляр через IPC (контракт п.10).
    match ipc_send(IpcCommand::Status).await {
        Ok(Some(resp)) if resp.ok => {
            let d = resp.data.unwrap_or_default();
            println!("Экземпляр: работает");
            println!("Движок:   {}", d["engine"].as_str().unwrap_or("?"));
            println!(
                "Профиль:  {}",
                d["active_profile"].as_str().unwrap_or("не выбран")
            );
            EXIT_OK
        }
        Ok(Some(resp)) => {
            eprintln!("mxds: {}", resp.error.unwrap_or_default());
            EXIT_ERROR
        }
        Ok(None) => {
            println!("Экземпляр: не запущен (mxds run)");
            EXIT_OK
        }
        Err(e) => {
            eprintln!("mxds: IPC: {e}");
            EXIT_ERROR
        }
    }
}

async fn cmd_stop() -> i32 {
    match ipc_send(IpcCommand::Stop).await {
        Ok(Some(resp)) if resp.ok => {
            println!("Движок остановлен");
            EXIT_OK
        }
        Ok(Some(resp)) => {
            eprintln!("mxds: {}", resp.error.unwrap_or_default());
            EXIT_ERROR
        }
        Ok(None) => {
            eprintln!("mxds: работающий экземпляр не найден");
            EXIT_ERROR
        }
        Err(e) => {
            eprintln!("mxds: IPC: {e}");
            EXIT_ERROR
        }
    }
}

async fn cmd_run(profile: Option<String>) -> i32 {
    // Движок требует прав администратора.
    if !crate::windows::permissions::is_elevated() {
        eprintln!("mxds: запуск движка требует прав администратора");
        return EXIT_NO_RIGHTS;
    }
    // Уже работающий экземпляр — команда через IPC.
    match ipc_send(IpcCommand::Run).await {
        Ok(Some(resp)) if resp.ok => {
            println!("Движок уже работает (команда передана экземпляру)");
            return EXIT_OK;
        }
        Ok(Some(resp)) => {
            eprintln!("mxds: {}", resp.error.unwrap_or_default());
            return EXIT_ERROR;
        }
        Ok(None) => {} // автономный режим
        Err(e) => {
            eprintln!("mxds: IPC: {e}");
            return EXIT_ERROR;
        }
    }

    // Эксклюзивный режим (Цикл 14): single-instance + очистка
    // осиротевшего драйвера WinDivert от прошлого краша.
    let core = match CoreManager::bootstrap_with(AppPaths::resolve(), BootMode::Exclusive).await {
        Ok(c) => std::sync::Arc::new(c),
        Err(e) => {
            if is_already_running(&e) {
                eprintln!("mxds: другой экземпляр уже запущен — используйте `mxds status` или `mxds stop`");
            } else {
                eprintln!("mxds: ошибка инициализации: {e}");
            }
            return EXIT_ERROR;
        }
    };
    if let Some(id) = &profile {
        if let Err(e) = core.select_profile(id.clone()).await {
            eprintln!("mxds: профиль {id}: {e}");
            return EXIT_ERROR;
        }
    }
    // Автоподбор профиля при явном --profile не применяем —
    // пользователь выбрал конкретный. Иначе — по настройке auto_select.
    if profile.is_none() && core.config().await.auto_select {
        match core.auto_select_profile().await {
            Ok(rep) => match &rep.selected {
                Some(id) => println!("Рабочий профиль подобран автоматически: {id}"),
                None => {
                    eprintln!(
                        "mxds: рабочий профиль не найден (попыток: {}); движок остановлен",
                        rep.attempts.len()
                    );
                    let _ = core.shutdown().await;
                    return EXIT_ERROR;
                }
            },
            Err(e) => {
                eprintln!("mxds: автоподбор: {e}");
                let _ = core.shutdown().await;
                return EXIT_ERROR;
            }
        }
    } else if let Err(e) = core.start_engine().await {
        eprintln!("mxds: запуск движка: {e}");
        return EXIT_ERROR;
    }
    println!("Движок запущен. Ctrl+C — остановка.");

    // IPC-сервер для управления этим экземпляром.
    let ipc = super::ipc_server::spawn(core.clone());

    // Ожидание Ctrl+C.
    let _ = tokio::signal::ctrl_c().await;
    println!("\nОстановка...");
    ipc.shutdown().await;
    if let Err(e) = core.shutdown().await {
        eprintln!("mxds: shutdown: {e}");
        return EXIT_ERROR;
    }
    EXIT_OK
}

async fn cmd_check() -> i32 {
    let core = match bootstrap_core().await {
        Ok(c) => c,
        Err(code) => return code,
    };
    println!("Проверка доступности (тайм-аут 5 с на этап)...");
    let results = core.check_connectivity(None).await;
    print_connectivity(&results);
    let _ = core.shutdown().await;
    EXIT_OK
}

fn print_connectivity(results: &[crate::connectivity::ConnectivityResult]) {
    let mut current = String::new();
    for r in results {
        if r.target != current {
            current = r.target.clone();
            println!("{current}");
        }
        let mark = match r.status {
            crate::diagnostics::CheckStatus::Pass => "[OK]",
            crate::diagnostics::CheckStatus::Warn => "[?]",
            crate::diagnostics::CheckStatus::Fail => "[X]",
            crate::diagnostics::CheckStatus::Blocked => "[-]",
        };
        println!(
            "  {:5} {:6} {:>6} мс  {}",
            mark,
            format!("{:?}", r.stage),
            r.elapsed.as_millis(),
            r.detail
        );
    }
}

async fn cmd_diagnostics(fix: bool) -> i32 {
    let core = match bootstrap_core().await {
        Ok(c) => c,
        Err(code) => return code,
    };
    let mut results = core.run_diagnostics().await;

    // --fix: применяем доступные авто-исправления и повторяем прогон,
    // чтобы результат был виден сразу (как кнопки «Исправить» в GUI).
    if fix {
        let fixes: Vec<_> = results
            .iter()
            .filter(|r| r.status != crate::diagnostics::CheckStatus::Pass && r.fix.is_some())
            .filter_map(|r| r.fix.map(|f| (r.name.clone(), f)))
            .collect();
        if fixes.is_empty() {
            println!("авто-исправлений не требуется");
        } else {
            for (name, f) in fixes {
                match core.run_fix(f).await {
                    Ok(msg) => println!("[FIX] {name}: {msg}"),
                    Err(e) => eprintln!("[FIX] {name}: ошибка: {e}"),
                }
            }
            println!("--- повторная диагностика ---");
            results = core.run_diagnostics().await;
        }
    }

    let mut fails = 0usize;
    for r in &results {
        let mark = match r.status {
            crate::diagnostics::CheckStatus::Pass => "[OK]",
            crate::diagnostics::CheckStatus::Warn => "[!]",
            crate::diagnostics::CheckStatus::Fail => {
                fails += 1;
                "[X]"
            }
            crate::diagnostics::CheckStatus::Blocked => "[-]",
        };
        println!("{mark} {:22} {}", r.name, r.description);
        if let Some(d) = &r.details {
            println!("       {d}");
        }
        if let Some(rec) = &r.recommendation {
            println!("       → {rec}");
        }
    }
    let _ = core.shutdown().await;
    if fails > 0 {
        eprintln!("mxds: диагностика: {fails} проверок FAIL");
        EXIT_DIAG_FAILED
    } else {
        EXIT_OK
    }
}

async fn cmd_config(cmd: ConfigCommands) -> i32 {
    let core = match bootstrap_core().await {
        Ok(c) => c,
        Err(code) => return code,
    };
    let code = match cmd {
        ConfigCommands::Show => match serde_json::to_string_pretty(&core.config().await) {
            Ok(json) => {
                println!("{json}");
                EXIT_OK
            }
            Err(e) => {
                eprintln!("mxds: {e}");
                EXIT_ERROR
            }
        },
        ConfigCommands::Validate => {
            let issues = core.validate_config().await;
            if issues.is_empty() {
                println!("Конфигурация валидна");
                EXIT_OK
            } else {
                let mut errors = 0;
                for i in &issues {
                    let sev = match i.severity {
                        crate::strategy::IssueSeverity::Error => {
                            errors += 1;
                            "ERROR"
                        }
                        crate::strategy::IssueSeverity::Warning => "WARN",
                    };
                    println!("{sev} {}: {}", i.field, i.message);
                }
                if errors > 0 {
                    EXIT_ERROR
                } else {
                    EXIT_OK
                }
            }
        }
    };
    let _ = core.shutdown().await;
    code
}

async fn cmd_profiles(cmd: ProfilesCommands) -> i32 {
    let core = match bootstrap_core().await {
        Ok(c) => c,
        Err(code) => return code,
    };
    let code = match cmd {
        ProfilesCommands::List => {
            let profiles = core.list_profiles().await;
            for p in &profiles {
                let active = if p.is_active { "*" } else { " " };
                let mark = compat_mark(p.compat);
                println!(
                    "{active} {:24} {} {:10} правил: {:2}  {}",
                    p.id,
                    mark,
                    format!("{:?}", p.compat),
                    p.rules_count,
                    p.name
                );
                if let Some(note) = &p.compat_note {
                    if !note.is_empty() {
                        println!("    └─ {note}");
                    }
                }
            }
            println!(
                "\n* — активный профиль; ✓ проверен / ~ частично / ✗ не работает / ? не тестирован"
            );
            EXIT_OK
        }
        ProfilesCommands::Info { id } => match core.profile_info(&id).await {
            Some(p) => {
                println!("Профиль:    {}", p.id);
                println!("Название:   {}", p.name);
                println!("Описание:   {}", p.description);
                println!("Совместимость: {:?}", p.compat);
                if let Some(note) = &p.compat_note {
                    println!("Заметка:    {note}");
                }
                println!("Правил:     {}", p.rules_count);
                println!("Активный:   {}", if p.is_active { "да" } else { "нет" });
                EXIT_OK
            }
            None => {
                eprintln!("mxds: профиль '{id}' не найден");
                EXIT_ERROR
            }
        },
        ProfilesCommands::Create {
            id,
            from,
            name,
            description,
        } => match core.create_profile(&id, from, name, description).await {
            Ok(p) => {
                println!(
                    "Профиль '{}' создан ({} правил). Проверьте его: scripts/profile-sweep.ps1",
                    p.id, p.rules_count
                );
                EXIT_OK
            }
            Err(e) => {
                eprintln!("mxds: создание профиля: {e}");
                EXIT_ERROR
            }
        },
        ProfilesCommands::Mark { id, status, note } => {
            let Some(cs) = parse_compat_status(&status) else {
                eprintln!(
                    "mxds: неизвестный статус '{status}' (ожидается: ok | partial | fail | reset)"
                );
                return EXIT_ERROR;
            };
            match core
                .mark_profile_compat(&id, cs, note.unwrap_or_default())
                .await
            {
                Ok(()) => {
                    println!("Профиль '{id}': статус {cs:?} сохранён");
                    EXIT_OK
                }
                Err(e) => {
                    eprintln!("mxds: mark: {e}");
                    EXIT_ERROR
                }
            }
        }
    };
    let _ = core.shutdown().await;
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_all_commands() {
        // Разбор аргументов без выполнения (try_parse_from).
        for args in [
            vec!["mxds", "status"],
            vec!["mxds", "check"],
            vec!["mxds", "diagnostics"],
            vec!["mxds", "config", "show"],
            vec!["mxds", "config", "validate"],
            vec!["mxds", "profiles", "list"],
            vec!["mxds", "profiles", "info", "general"],
            vec![
                "mxds",
                "profiles",
                "create",
                "my-prof",
                "--from",
                "general-alt6",
            ],
            vec!["mxds", "profiles", "create", "my-prof2"],
            vec![
                "mxds",
                "profiles",
                "mark",
                "general",
                "ok",
                "--note",
                "youtube 200",
            ],
            vec!["mxds", "run"],
            vec!["mxds", "run", "--profile", "general-alt"],
            vec!["mxds", "stop"],
            vec!["mxds", "version"],
        ] {
            Cli::try_parse_from(&args).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        }
    }

    #[test]
    fn cli_rejects_unknown() {
        assert!(Cli::try_parse_from(["mxds", "wipe"]).is_err());
        assert!(Cli::try_parse_from(["mxds", "profiles"]).is_err());
    }

    #[test]
    fn compat_status_parsing() {
        use crate::strategy::CompatStatus as C;
        assert_eq!(parse_compat_status("ok"), Some(C::Supported));
        assert_eq!(parse_compat_status("PARTIAL"), Some(C::PartiallySupported));
        assert_eq!(parse_compat_status("fail"), Some(C::Unsupported));
        assert_eq!(parse_compat_status("reset"), Some(C::NotTested));
        assert_eq!(parse_compat_status("nonsense"), None);
    }
}
