# Потоки данных MXDS

Дата: 2026-11-13. Статус: документ Цикла 3.

## 1. Запуск приложения

1. `main.rs`: парсинг аргументов (есть args → CLI, нет → GUI).
2. SingleInstance: мьютекс `Global\MXDS.SingleInstance`.
   - GUI-режим при занятом мьютексе → активация существующего окна через
     IPC, выход.
   - CLI-режим → попытка IPC к работающему экземпляру; при отказе —
     автономный режим.
3. Bootstrap Core: ConfigManager::load (default.json → config.json →
   ENV) → валидация → StrategyManager::discover → проверка
   восстановления (осиротевший WinDivert, lock-файл мёртвого PID).
4. Событие `ApplicationStarted`. GUI/CLI отображают фактическое состояние.

## 2. Старт движка (главный пользовательский сценарий)

```
GUI/CLI → Core.start_engine()
  → проверка прав (is_elevated)             ── отказ → EngineError::AccessDenied
  → конфликтующие службы                     ── найдены → Blocked + рекомендация
  → StrategyManager::resolve(active_profile) ── невалиден → StrategyError
  → EngineManager::start(profile, cfg)
      → DivertHandle::open(filter)           ── ошибки по таблице
                                               WINDIVERT_INTEGRATION.md п.6
      → проверка driver_version == 2.x
      → SetParam(queue...) по конфигу
      → spawn рабочего потока
  → state = Running (только после первого успешного Recv)
  → AppEvent::EngineStarted
```

## 3. Рабочий цикл движка

```
WinDivertRecv ──→ parse (HelperParsePacket)
  │                    │
  │                    ▼
  │        профиль: правила по (proto, порт, L7, hostlist, ipset)
  │                    │
  │         ┌──────────┼───────────────┐
  │         ▼          ▼               ▼
  │      не цель    техника fake    техника split/disorder
  │         │       (payload ×N,    (split-pos, seqovl,
  │         │        fooling)        seqovl-pattern)
  │         │          │               │
  │         ▼          ▼               ▼
  │      WinDivertSend (оригинал / фейки / фрагменты)
  │
  └── статистика (recv/sent/dropped) → периодически в Core
```

- Пакеты с `Impostor=1` от собственных инъекций отбрасываются от
  обработки (защита от рециркуляции).
- Ошибка Recv (не NO_DATA при штатном shutdown) → `EngineFailed`,
  состояние FAILED, событие в шину.

## 4. Проверка доступности

```
GUI/CLI → Core.check_connectivity(targets?)
  → targets: Discord, YouTube, пользовательские, из списков
  → каскад DNS → TCP → TLS → HTTP (каждый этап: статус + время)
  → результаты → data/connectivity-history.jsonl
  → AppEvent::ConnectivityCheckCompleted
```

Важно: проверки выполняются СКВОЗЬ работающий движок (это и есть
проверка эффективности стратегии) и могут выполняться при остановленном
движке (базовая линия).

## 5. Диагностика

```
Core.run_diagnostics()
  → default_checks(): права, ОС, BFE, файлы WinDivert, версия драйвера,
    timestamps, прокси, DoH, конфликты служб, кириллица/OneDrive, hosts,
    целостность конфига, состояние движка
  → Vec<CheckResult> → AppEvent::DiagnosticCompleted
```

Исправления (timestamps, удаление конфликтующих служб, очистка кэша
Discord) — отдельные команды, требующие явного подтверждения в GUI/CLI.

## 6. Смена профиля

```
Core.select_profile(id)
  → валидация профиля (payloads существуют, синтаксис)
  → сохранение в config (атомарно)
  → если движок Running: контролируемый перезапуск
    (stop → start с новым профилем); при провале старта — FAILED,
    откат профиля НЕ выполняется автоматически (пользователь видит ошибку)
  → AppEvent::ProfileChanged
```

## 7. Остановка и завершение

```
Core.stop_engine() → EngineManager::stop()
  → DivertHandle::shutdown(RECV) → дочитать очередь → закрыть поток
  → DivertHandle::drop → WinDivertClose
  → state = Stopped, AppEvent::EngineStopped

Core.shutdown() → stop_engine (если Running) → сохранение состояния
  → закрытие Event Bus → ApplicationStopped
```

При SIGINT/kill: Drop-гарантии закрывают хендлы; драйвер снимает фильтры
при закрытии хендла — система не остаётся с «застрявшим» перехватом.

## 8. Восстановление после сбоя

При следующем старте bootstrap обнаруживает: службу WinDivert без
процесса-владельца (sc query) → предлагает очистку; lock-файл с мёртвым
PID → снимается; повреждённый config.json → восстановление из .bak
с WARN-событием.
