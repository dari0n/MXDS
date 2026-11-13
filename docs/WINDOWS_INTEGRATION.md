# Windows Integration

Дата: 2026-11-13. Статус: аналитический документ Цикла 2.
Область: модуль `src/windows/` — изолированный слой ОС-интеграции MXDS.

## 1. Целевая платформа

- Windows 10 x64 (сборки 1809+) и Windows 11 x64.
- Хост разработки: Windows 11, build 10.0.26300 (проверено 2026-11-13).
- Определение версии: `RtlGetVersion` / реестр
  `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`
  (ProductName, CurrentBuild, UBR, DisplayVersion).

## 2. Права администратора

- Требуются для: `WinDivertOpen` (загрузка драйвера), управления службами
  (SCM), `netsh interface tcp set global`, записи в hosts.
- Проверка: `OpenProcessToken` + `TokenElevation` (через windows-sys),
  НЕ через «попробовать и упасть».
- Политика минимизации привилегий: GUI запускается без elevation и
  предлагает перезапуск с правами только для операций, которым они нужны
  (старт/стоп движка, установка службы). Модель: единый процесс с
  манифестом `requireAdministrator` ИЛИ разделение на непривилегированный
  UI + привилегированный движок — решается в Цикле 3 (архитектура),
  кандидат по умолчанию: манифест `requireAdministrator` для всего
  приложения (проще, соответствует UX «запустил и забыл»).

## 3. Service Control Manager

Операции (обёртки над windows-sys, не sc.exe):
- `OpenSCManager` / `OpenService` / `CreateService` / `StartService` /
  `ControlService(STOP)` / `DeleteService` / `QueryServiceStatus`.
- Служба MXDS: имя `mxds` (не `zapret`, чтобы не конфликтовать с
  оригиналом), DisplayName «MXDS», start=auto при установке автозапуска.
- Имя активной стратегии: хранить в конфиге MXDS (не в реестре, как
  оригинал) — реестр не нужен.
- Службы WinDivert (`WinDivert`, `WinDivert14`): только остановка/удаление
  при деинсталляции или лечении конфликтов (см. диагностику оригинала F-11).

## 4. Базовая сетевая подготовка

- **BFE (Base Filtering Engine)**: обязательна для WinDivert
  (иначе `EPT_S_NOT_REGISTERED`). Проверка `QueryServiceStatus(BFE)`.
- **TCP timestamps**: fooling `ts` требует `netsh interface tcp set global
  timestamps=enabled`. Реализация: NetioAPI/проверка через
  `netsh interface tcp show global` (чтение) и изменение только с
  подтверждения пользователя + запись в лог. Документировать как
  системное изменение.
- **Системный прокси**: чтение `HKCU\...\Internet Settings`
  (ProxyEnable/ProxyServer) — только диагностика, без изменений.
- **Secure DNS / DoH**: чтение `HKLM\...\Dnscache\InterfaceSpecificParameters`
  (DohFlags) — только диагностика.

## 5. Файловая интеграция

- Установка: директория без кириллицы и спецсимволов (диагностика;
  известная проблема оригинала); предупреждение при OneDrive.
- Данные приложения: `config/`, `lists/`, `profiles/`, `assets/`,
  `logs/` рядом с exe (portable) — соответствует «запустил и забыл»;
  запись в Program Files невозможна без прав, поэтому установщик (Цикл 16)
  либо кладёт в `%ProgramFiles%\MXDS` с данными в `%ProgramData%\MXDS`,
  либо portable — решение в Цикле 16.
- Атомарная запись файлов: write-to-temp + rename (MoveFileEx с
  MOVEFILE_REPLACE_EXISTING).

## 6. Обработка системных ошибок

- Все WinAPI-вызовы возвращают `Result<_, WindowsError>`; код ошибки
  (`GetLastError`/`HRESULT`) + контекст операции + рекомендация.
- Маппинг известных кодов в понятные сообщения (таблица ошибок
  WinDivertOpen — см. WINDIVERT_INTEGRATION.md, раздел 6).
- Ни один системный вызов не паникует; отказ компонента переводит
  приложение в состояние FAILED с диагностикой.

## 7. Управление жизненным циклом и восстановление

- Единственный экземпляр: именованный мьютекс (`CreateMutex`,
  `Global\MXDS.SingleInstance`) + lock-файл с PID и проверкой живости
  процесса. Локальная координация CLI↔GUI: именованный канал
  (Named Pipe) с ACL текущего пользователя — детальный дизайн в Цикле 3,
  соответствует запрету HTTP API (PROMPT.md, раздел 9).
- Восстановление после сбоя: при старте проверяется «осиротевший»
  драйвер WinDivert (служба активна, процесса нет) → остановка/удаление
  службы WinDivert, как диагностика оригинала.
- Обнаружение конфликтующих обходов: запросы SCM по списку
  (GoodbyeDPI, discordfix_zapret, winws1/2, zapret) → статус BLOCKED
  с рекомендацией; удаление только по явному подтверждению.

## 8. Деинсталляция

Порядок (соответствует Remove Services оригинала, но без taskkill):
остановить движок (graceful) → удалить службу mxds → остановить и
удалить службы WinDivert/WinDivert14 → удалить файлы. Остаточный
драйвер WinDivert выгружается перезагрузкой (поведение Windows,
документируется пользователю).

## 9. Ограничения Windows 10/11

- Подпись драйвера обязательна (Secure Boot / HVCI не меняют этого для
  подписанного WinDivert 2.2; неподписанные драйверы не загружаются).
- Антивирусы могут блокировать драйвер (ERROR_DRIVER_BLOCKED 1275).
- Виртуальные среды без поддержки драйверов: ERROR_DRIVER_BLOCKED.
- Windows 7 вне области (оригинал требовал особые подписанные бинарники).
