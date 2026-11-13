# Changelog

Все заметные изменения проекта MXDS фиксируются в этом файле.

Формат: [Semantic Versioning](https://semver.org/lang/ru/), MAJOR.MINOR.PATCH.

## [Unreleased]

### Added (Автоподбор профиля и доверенные исправления) — 2026-11-13
- Одноразовая подсказка при первом запуске GUI: модальное окно про
  автоподбор (первый перебор — до пары минут), маркер
  `data/.autoselect-hint-shown`; показывается, только если автоподбор
  включён.
- Автоподбор профиля при запуске движка (config `auto_select`, по
  умолчанию вкл.): активный профиль проверяется первым, затем кандидаты
  по статусу совместимости (рабочие → непроверенные → частичные →
  нерабочие), максимум 8 попыток. Для каждого кандидата — запуск
  движка и каскад доступности YouTube; первый рабочий становится
  активным, результаты отмечаются в data/profile-compat.json,
  прогресс — событиями AppEvent::AutoSelectProgress (баннер в GUI).
  При полном провале движок останавливается, выбор откатывается.
  Работает в GUI (кнопка «Запустить») и CLI (`mxds run`; при явном
  `--profile` автоподбор не применяется). Переключатель на экране
  «Конфигурация».
- Доверенные исправления диагностики: `mxds diagnostics --fix`
  применяет доступные исправления и сразу повторяет прогон (как
  кнопки «Исправить» в GUI). E2E-автотест цикла
  диагностика→исправление→повторная диагностика (tests/diagnostics_fix.rs,
  самопропуск без прав администратора).
- Secure DNS: исправление теперь реально включает DoH — регистрирует
  шаблоны (Cloudflare/Google/Quad9), включает DoH (DohFlags=1) на
  адаптерах с известным DoH-резолвером, DHCP-адаптеры переключает на
  1.1.1.1/1.0.0.1 (статические DNS не трогает, сообщает в итоге),
  результат контролируется повторной проверкой. Проверка
  secure_dns_configured учитывает DohInterfaceSettings\<IP>\DohFlags
  (реальное расположение флагов Windows 11).
- Маркеры статусов в CLI-диагностике унифицированы с GUI ([!] для Warn).

### Added (GUI: помощь и авто-исправления диагностики) — 2026-11-13
- Экран «Помощь» в GUI: быстрый старт, создание профилей (GUI/CLI/
  вручную), диагностика и фиксы, справка по CLI, типовые проблемы.
- Авто-исправления диагностики: `FixId` в `CheckResult` для исправимых
  проверок (TCP timestamps, Secure DNS, осиротевший WinDivert,
  конфликтующие службы); `diagnostics::run_fix` и
  `CoreManager::run_fix` (требуют elevation).
- GUI «Диагностика»: кнопка «Исправить» напротив исправимых проверок
  и «Исправить все (N)»; после исправления прогон перезапускается
  автоматически. Без прав администратора кнопки неактивны.
- Secure DNS: регистрация DoH-шаблонов Cloudflare/Google/Quad9 через
  Set-DnsClientDohServerAddress (AutoUpgrade + AllowFallbackToUdp).
- Зелёный информационный баннер (CoreEvent::Info) отдельно от красного
  баннера ошибок; создание профиля теперь сообщает успех через Info.
- Рефакторинг текстов GUI: общий маркер статусов check_mark для
  «Диагностики» и «Доступности», исправлены подписи и формулировки.
- Интеграционный тест strategy_profiles больше не зависит от локального
  data/profile-compat.json (реальные результаты прогонов).

### Added (Profile Management) — 2026-11-13
- Управление профилями: создание через CLI (`mxds profiles create <id>
  [--from <база>] [--name …] [--description …]`) и GUI (экран «Профили» →
  «Создать профиль»: копия существующего или пустой шаблон, slug-проверка).
- Статусы совместимости: `mxds profiles mark <id> ok|partial|fail|reset
  [--note …]` и клик по бейджу в GUI; хранятся в
  `data/profile-compat.json` (атомарная запись), накладываются на профили
  при discover/load, не меняя файлы профилей. Бейджи: ✓/~ /✗/? в CLI
  и GUI, заметка — в подсказке бейджа и `profiles list/info`.
- Событие AppEvent::ProfilesChanged → GUI обновляет список;
  CoreCommand::RefreshProfiles/CreateProfile/MarkCompat.
- docs/PROFILE_GUIDE.md: формат профиля, создание (GUI/CLI/вручную),
  проверка перебором, отметка результатов, рекомендации.
- Скрипты проверки профилей на живой сети: profile-sweep.ps1 (все
  профили), verify-profiles.ps1 (точечно), discord-check.ps1.
- Тесты: CompatStore roundtrip/деградация/дата, создание/дубликаты/
  slug, mark + событие + персистентность, CLI-парсинг create/mark.

### Added (Cycle 16) — 2026-11-13
- Release-дистрибутив: scripts/build-release.ps1 — cargo --release,
  стейджинг `dist/MXDS-<версия>-win-x64/` (mxds.exe, assets/, config/,
  profiles/, lists/, docs/, README, скрипты), SHA256SUMS.txt (77 файлов),
  ZIP + .sha256. Скрипты дистрибутива принудительно UTF-8 с BOM
  (PowerShell 5.1).
- Установщик scripts/install.ps1: проверка зависимостей (x64,
  Windows build ≥ 17763), копирование, проверка целостности при
  установке, опциональный автозапуск через Планировщик задач
  (`-Autostart`, при входе, наивысшие права).
- Удаление scripts/uninstall.ps1: остановка экземпляра, снятие
  автозапуска, очистка осиротевших служб WinDivert/WinDivert14,
  удаление каталога (`-KeepData` сохраняет config.json и logs/).
- Проверка целостности scripts/verify-install.ps1: сверка с
  SHA256SUMS.txt, коды выхода 0/1.
- ACCEPTANCE_REPORT.md: приёмка release-сборки, установки и удаления.
- README: раздел «Установка и удаление», обновлён статус разработки.

### Fixed (Cycle 16) — 2026-11-13
- PowerShell 5.1: BOM для кириллицы; `${var}:` вместо `$var:` в строках;
  `$PSScriptRoot` в значении параметра по умолчанию при
  `[CmdletBinding()]` пуст → резолв в теле verify-install.ps1;
  uninstall.ps1 уходит из удаляемого каталога (CWD блокировал удаление).

### Added (Cycle 15) — 2026-11-13
- tests/e2e.rs: 8 End-to-End тестов собранного бинарника
  (CARGO_BIN_EXE_mxds, изолированный MXDS_ROOT): version; неизвестная
  подкоманда → код 2; status без работающего экземпляра; profiles
  list; config show — валидный JSON; config validate; schema 999 →
  код 1 без паники; run без прав администратора → код 4.
- GUI-тесты: apply_event (состояние движка, сохранение черновика
  конфигурации при новых событиях, баннеры Error/Fatal, списки),
  read_logs (пустой журнал, хвост N строк новейшего файла).
- Тесты восстановления: bootstrap_with(Exclusive) при удержанном
  single-instance → InstanceAlreadyRunning; Plain не конфликтует.
  Сериализация тестов глобального мьютекса — SI_TEST_LOCK.
- Регрессионный тест rotator: после ротаций не остаётся *.rot-tmp.

### Changed (Cycle 15) — 2026-11-13
- gui/app.rs: drain_events разделён на цикл приёма + apply_event
  (тестируемость без запуска core-потока).

### Added (Cycle 14) — 2026-11-13
- Ротация логов по размеру: logging/rotator.rs (SizeRotatingWriter,
  активный mxds.log, архивы mxds.log.1…5, лимит 10 МиБ на файл);
  заменил daily-аппендер в logging::init, при ошибке файла —
  деградация до stderr-only.
- Уровень логирования в main.rs — из конфигурации (best-effort,
  fallback Info), файловые логи в <root>/logs, лог версии при старте.
- Восстановление после неожиданного завершения:
  CoreManager::bootstrap_with(paths, BootMode::Exclusive) — захват
  single-instance мьютекса + best-effort очистка осиротевшего
  драйвера WinDivert; CLI run и GUI переведены на Exclusive,
  понятные сообщения при уже запущенном экземпляре.
- SingleInstance: Send+Sync (ядерный HANDLE) для хранения в Core.
- Tracing-сводки: диагностика (total/pass/warn/fail), проверка
  доступности (targets/ok/failed).

### Changed (Cycle 14) — 2026-11-13
- Cargo.toml [profile.release]: panic = "unwind" (вместо abort) —
  иначе не работают panic-hook GUI и изоляция паник диагностики.

### Added (Cycle 13) — 2026-11-13
- GUI (eframe 0.31, glow): экраны Главная, Движок, Профили,
  Конфигурация, Доступность, Диагностика, Логи, О программе
  (PROMPT.md Цикл 13).
- Архитектура: GUI-поток ↔ фоновый поток с tokio-рантаймом
  (владеет CoreManager и IPC-сервером); обмен mpsc-каналами
  CoreCommand/CoreEvent; бизнес-логики в GUI нет.
- Главная: состояния STOPPED/STARTING/RUNNING/STOPPING/FAILED
  (цветной индикатор), активный профиль, WinDivert, доступность
  Discord/YouTube, последние ошибки, кнопки запуска/остановки.
- Движок: состояние, статистика пакетов (тик 1 с). Профили:
  список с выбором (●/○). Конфигурация: черновик с редактированием
  (уровень логов, Game Filter, очередь, обновления), сохранение
  через CoreManager::update_config. Доступность и Диагностика:
  запуск прогона + цветные результаты. Логи: хвост журнала.
- panic-hook GUI (ERROR_HANDLING п.7), graceful shutdown при выходе
  (CoreCommand::Shutdown → остановка движка и IPC).
- Баннер «без прав администратора» при запуске без elevation.
- main.rs: без аргументов — GUI; CLI без изменений.

### Added (Cycle 12) — 2026-11-13
- CLI (clap derive): status, check, diagnostics, config show/validate,
  profiles list/info, run [--profile], stop, version. Коды выхода
  0/1/2/3/4 по ERROR_HANDLING п.6 (3 — FAIL в диагностике, 4 — нет
  прав администратора для run).
- Named Pipe IPC (D-009): windows::ipc — транспорт \\.\pipe\MXDS.Control,
  JSON-lines, DACL «только текущий пользователь» (SDDL по SID из
  токена), лимит сообщения 1 МиБ.
- cli/protocol.rs: allowlist команд IpcCommand (status/stop/run/
  get-config/logs-tail), произвольное выполнение невозможно
  по построению; неизвестные команды отклоняются.
- cli/ipc_server.rs: сервер в процессе владельца CoreManager;
  корректная остановка — dummy-подключение будит блокирующий
  accept (иначе поток spawn_blocking не даёт рантайму завершиться).
- cli/ipc_client.rs: status/stop/run сначала к работающему
  экземпляру; при отсутствии — автономный Core (контракт п.10).
- CoreManager: validate_config, profile_info; AppPaths::resolve
  (MXDS_ROOT → exe-каталог → dev-корень).
- main.rs: полный CLI, tokio-рантайм, graceful Ctrl+C.
- 12 новых тестов (всего 127 unit-тестов), включая сквозной
  IPC сервер↔клиент и отказ по allowlist.

### Added (Cycle 11) — 2026-11-13
- Connectivity Checker: каскад DNS→TCP→TLS→HTTP на цель
  (провал этапа фиксируется, вышестоящие — Blocked), ICMP ping
  (IcmpSendEcho, IPv4), тайм-аут на каждый этап, параллельный
  прогон целей (JoinSet), порядок результатов = порядку целей.
- TLS — rustls (ring) + webpki-roots; HTTP — GET со статус-строкой
  (любой валидный код = Pass, код в detail).
- Цели: config/targets.txt (формат оригинала, 16 целей Discord/YouTube/
  Google/Cloudflare/DNS), встроенный fallback, парсер с пропуском
  битых строк.
- История: append-only JSON-lines data/connectivity-history.jsonl,
  ротация по размеру 5 МиБ (один бэкап .1).
- CoreManager::check_connectivity (по контракту п.2) + событие
  ConnectivityCheckCompleted.
- Тесты CoreManager изолированы от рабочего config.json: каждый тест
  получает временную копию корня (default.json, profiles, lists,
  assets) — устранена гонка параллельного прогона.
- 10 новых тестов (всего 115 unit-тестов; ping 1.1.1.1 — #[ignore]).

### Added (Cycle 10) — 2026-11-13
- Diagnostics: DiagContext (пути + снимок состояния движка/конфигурации),
  трейт Check с run(ctx), default_checks (20 проверок), run_all
  с замером длительности и изоляцией паник (catch_unwind).
- Проверки (перенос Run Diagnostics оригинала, F-11): elevation,
  os_version, bfe_service, windivert_files (существование + SHA256),
  windivert_driver (остаточный драйвер), tcp_timestamps, system_proxy,
  secure_dns, conflicting_services, adguard/killer/intel_connectivity/
  check_point/smartbyte/vpn (по именам служб), cyrillic_path,
  onedrive_path, hosts_file (youtube-записи), config_integrity,
  engine_state.
- windows::components::enumerate_services — перечисление Win32-служб
  через EnumServicesStatusExW (SCM API, без process-spawn).
- CoreManager::run_diagnostics (spawn_blocking + событие
  DiagnosticCompleted).
- 11 новых тестов (всего 105 unit-тестов).

### Added (Cycle 9) — 2026-11-13
- Application Core: CoreManager — центральный владелец бизнес-состояния
  (docs/MODULE_CONTRACTS.md п.2): bootstrap (config → strategy →
  engine → состояние), start_engine/stop_engine (идемпотентные),
  engine_stats, list_profiles/select_profile (валидация профиля,
  персист в конфигурацию, перезапуск работающего движка),
  update_config (отклонение невалидной, перезапуск при смене
  engine-параметров/Game Filter, синхронизация active_profile
  в AppState), shutdown (идемпотентный), state, subscribe.
- Типы application: AppPaths, ProfileInfo, CoreError
  (Shutdown/NoActiveProfile/Mxds).
- 8 новых tokio-тестов CoreManager (всего 94 unit-теста).

### Added (Cycle 8) — 2026-11-13
- Strategy Manager: discover/validate/load/resolve профилей
  (profiles/*.json, StrategyManager).
- Парсер winws-аргументов .bat → типизированный Profile
  (extract_winws_args, parse_winws_args); сбор неподдержанных ключей.
- Типы профиля: DesyncSpec, DesyncTechnique (fake/multisplit/
  multidisorder/syndata/hostfakesplit/fakedsplit), SplitPos
  (absolute/sniext+N/midsld), FoolingKind (ts/badseq/md5sig), cutoff,
  any-protocol, fake-payload'ы по L7, hostlist/ipset (файлы и встроенные
  домены, включения и исключения), ip-id zero.
- lists.rs: HostMatcher (точные ^ + суффиксные + исключения),
  IpMatcher (CIDR→интервалы, слияние, бинарный поиск).
- engine/rule.rs: EngineRule, classify (TLS/QUIC/HTTP/DiscordStun/
  Unknown), FlowTable (cutoff с TTL и GC), resolve_split_positions,
  apply (repeats, seqovl, disorder, ip-id zero).
- engine/sni.rs: извлечение SNI из TLS ClientHello и Host из HTTP.
- desync.rs: split_tcp_multi, split_tcp_seqovl, corrupt_ts_option,
  zero_ip_id, badseq_increment.
- 22 профиля оригинала импортированы в profiles/*.json
  (compat=NOT_TESTED), утилита src/bin/mxds_import.rs.
- Списки lists/ из оригинала (D-004): list-general, list-google,
  list-exclude, ipset-all, ipset-exclude.
- 31 новый юнит-тест (всего 89) + интеграционный strategy_profiles.

### Added (Cycle 7) — 2026-11-13
- Rust Network Engine: собственный сетевой движок поверх WinDivert,
  без winws.exe.
- assets/windivert (WinDivert.dll + WinDivert64.sys) и 24 payload'а
  assets/payloads (tls_clienthello_*, quic_initial_*, stun*, ACTIVE_*),
  SHA256 зафиксированы (D-004).
- windivert/ffi.rs: repr(C) layout'ы (compile-time asserts), загрузка
  DLL через libloading с проверкой SHA256 dll+sys.
- windivert/adapter.rs: DivertHandle (open/recv/send/set_param/
  driver_version/shutdown, Drop→Close), DivertParam, MAX_PACKET_SIZE.
- engine/packet.rs: чистый парсер IPv4/IPv6/TCP/UDP, детекторы TLS
  ClientHello и QUIC Initial, разбор extension-заголовков IPv6.
- engine/filter.rs: генератор строки фильтра WinDivert из портов TCP/UDP
  и GameFilter (режимы all/tcp/udp, диапазоны).
- engine/desync.rs: DesyncMode (None/Fake/Split/FakeSplit/Disorder),
  Fooling (ttl/bad_checksum/bad_seq), build_fake_tcp/udp, split_tcp.
- engine/manager.rs: EngineManager (start/stop/state/stats),
  рабочий OS-поток, graceful stop (Shutdown→drain→Close), EnginePlan.
- tests/engine_live.rs: 4 live-теста с реальным драйвером (#[ignore],
  elevated) — все пройдены; scripts/run-live-tests.bat.

### Fixed (Cycle 7) — 2026-11-13
- QUEUE_SIZE передаётся в байтах (поставляемая сборка WinDivert
  zapret-win-bundle отклоняет КБ-значения с кодом 87).

### Added (Cycle 6) — 2026-11-13
- Windows Integration (windows-sys 0.59): модуль windows/ реализован.
- permissions: is_elevated / require_elevated (OpenProcessToken +
  GetTokenInformation, безопасная деградация в false).
- integration: os_version (RtlGetVersion + UBR из реестра),
  is_supported_os (build >= 17763), system_proxy, secure_dns_configured
  (DohFlags в Dnscache), tcp_timestamps_enabled / enable_tcp_timestamps
  (netsh, английские и русские маркеры вывода), bfe_running,
  path_has_cyrillic, is_onedrive_path; приватные реестр-хелперы.
- components: ServiceStatus / service_status, conflicting_services
  (GoodbyeDPI, discordfix_zapret, winws1/2, zapret), stop_and_delete,
  windivert_cleanup (WinDivert / WinDivert14), ServiceManager
  (install/start/stop/remove службы mxds, RAII-хендлы SCM),
  SingleInstance (мьютекс Global\MXDS.SingleInstance + lock-файл с PID
  и проверкой живости владельца).
- 8 новых unit-тестов (всего 34). clippy: 0 предупреждений.

### Added (Cycle 5) — 2026-11-13
- Config Manager: загрузка default.json → config.json → ENV (MXDS_*),
  рекурсивное слияние JSON, валидация (порты Game Filter — перенос
  validate_game_filter_range оригинала; queue-параметры WinDivert),
  атомарное сохранение с резервной копией .bak, восстановление из .bak,
  версионирование схемы и механизм миграций, ConfigManager.update
  с отклонением невалидной конфигурации.
- config/default.json — эталонная конфигурация дистрибутива.
- 13 новых unit-тестов (всего 26).

## [0.1.0] — 2026-11-13 (Cycle 4)

### Added
- Базовая инфраструктура Rust: Cargo.toml, main.rs, lib.rs, 12 модулей
  (application, engine, windivert, windows, strategy, config,
  diagnostics, connectivity, cli, gui, logging, storage).
- Типы: MxdsError + доменные ошибки, EngineState (машина состояний),
  AppEvent/EventBus, Config/GameFilter/IpsetMode, ENV-парсеры MXDS_*,
  DivertError с маппингом кодов WinDivert, CheckResult/CheckStatus,
  ConnectivityResult/Stage/Target, atomic_write.
- Логирование tracing с ежедневной ротацией файлов (logs/mxds.log).
- 13 unit-тестов. Проверки: cargo check/test/clippy/fmt — OK.

### Added (Cycle 3) — 2026-11-13
- Архитектура: docs/ARCHITECTURE.md, docs/MODULE_CONTRACTS.md,
  docs/DATA_FLOW.md, docs/ERROR_HANDLING.md.
- Решения D-007 (пакет/бинарник/служба `mxds`, ENV `MXDS_*`),
  D-008 (единый elevated-процесс), D-009 (Named Pipe координация
  CLI↔GUI, без сетевых сокетов).
- README: раздел «Безопасность» (резюме SECURITY_REVIEW), таблица
  документации, блок лицензий.

### Added (Cycle 2) — 2026-11-13
- Анализ Windows и WinDivert: docs/WINDIVERT_INTEGRATION.md,
  docs/WINDOWS_INTEGRATION.md, docs/SECURITY_REVIEW.md.
- Проверены бинарники: WinDivert64.sys v2.2 (подпись валидна),
  WinDivert.dll (не подписана); хост Windows 11 build 26300.
- Зафиксированы лицензионные условия: WinDivert LGPL v3 (динамическая
  загрузка нетронутых бинарников), оригинал MIT (attribution).
- scripts/check-binaries.ps1 — проверка версий/подписей бинарников.

### Added (Cycle 1) — 2026-11-13
- Полный анализ оригинального проекта zapret-discord-youtube v1.10.4:
  docs/ORIGINAL_PROJECT_ANALYSIS.md, docs/FUNCTIONALITY_MAP.md,
  docs/DEPENDENCY_MAP.md, docs/STRATEGY_COMPATIBILITY.md.
- Зафиксирована матрица из 24 стратегий и техник десинхронизации DPI
  (fake, multisplit, multidisorder, syndata, hostfakesplit, fakedsplit,
  fooling ts/badseq, fake-tls-mod, cutoff, any-protocol).

## [0.0.0] — 2026-11-13

### Added
- Инициализация проекта (Цикл 0): структура каталогов, управляющие
  документы (context.md, CHANGELOG.md, ROADMAP.md, DECISIONS.md),
  README.md, .gitignore, локальный Git-репозиторий.
- PROMPT.md: скопировано ТЗ; внесена правка владельца — DPI-evasion-техники
  разрешены к реализации (разделы 2.2 и Цикл 7).
