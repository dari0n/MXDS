# Анализ оригинального проекта Flowseal/zapret-discord-youtube

Дата анализа: 2026-11-13
Источник: локальная копия `C:\Users\MINX\Documents\rust\divert\example`
(локальная версия по service.bat: `LOCAL_VERSION=1.10.4`)
Upstream: https://github.com/Flowseal/zapret-discord-youtube
Лицензия: MIT (Copyright (c) 2016-2026 bol-van, 2024-2026 Flowseal) — `LICENSE.txt`.

## 1. Назначение проекта

Набор Batch/PowerShell-скриптов-обёрток вокруг консольной утилиты
`winws.exe` (Windows-порт `nfqws` из проекта bol-van/zapret). Назначение —
обход DPI-блокировок (Discord, YouTube и др.) на Windows путём перехвата
трафика через драйвер WinDivert и десинхронизации DPI: подмена/фрагментация
пакетов, фейковые TLS ClientHello и QUIC Initial, нарушение нумерации
TCP sequence и т.п.

Flowseal-репозиторий сам по себе не содержит сетевого движка: вся обработка
трафика выполняется бинарником `winws.exe`, взятым из
[zapret-win-bundle](https://github.com/bol-van/zapret-win-bundle).
Скрипты отвечают за: запуск winws с параметрами стратегии, установку службы
Windows, диагностику, обновление списков и тестирование стратегий.

## 2. Состав проекта

| Путь | Назначение |
|---|---|
| `general.bat` + 23 варианта (ALT1-13, EXP, FAKE TLS AUTO[ ALT-ALT3], SIMPLE FAKE[ ALT-ALT2]) | Стратегии: наборы аргументов командной строки winws.exe |
| `service.bat` (1311 строк) | Интерактивное меню: установка/удаление службы, статус, переключатели Game Filter / IPSet Filter / автообновлений, замена активных фейков, обновление ipset и hosts, диагностика, запуск тестов |
| `bin/winws.exe` (203776 байт) | Сетевой движок (DPI desync). В MXDS запрещён |
| `bin/WinDivert.dll` (47616), `bin/WinDivert64.sys` (94144) | WinDivert — драйвер перехвата пакетов. Разрешён как системная зависимость |
| `bin/cygwin1.dll` (2954293) | Cygwin-рантайм для winws.exe. В MXDS не нужен |
| `bin/*.bin` (26 файлов, 100-1357 байт) | Бинарные payload'ы: фейковые TLS ClientHello (`tls_clienthello_*`), QUIC Initial (`quic_initial_*`), STUN (`stun*.bin`), активные фейки Discord/игр (`ACTIVE_DISCORD_UDP.bin`, `ACTIVE_GAME_UDP.bin`) |
| `lists/list-general.txt` (59 доменов) | Основной hostlist: Discord, YouTube, Cloudflare и др. Строки с `^` — точное совпадение хоста |
| `lists/list-google.txt` (22 домена) | Отдельный hostlist для Google/YouTube-инфраструктуры (отдельная стратегия обработки) |
| `lists/list-exclude.txt` (129 доменов) | Исключения из обработки (ростест, банки, госуслуги, VK, Ozon и т.д.) |
| `lists/ipset-all.txt` | IP/подсети для обхода. Пустой файл = режим `any`; запись `203.0.113.113/32` = режим `none`; иначе `loaded`. `.backup` (577 КБ) — резервная копия полного списка |
| `lists/ipset-exclude.txt` | Приватные/служебные диапазоны (RFC1918, CGNAT, multicast и т.п.) |
| `lists/*-user.txt` | Создаются при первом запуске: пользовательские hostlist/ipset/исключения |
| `utils/test zapret.ps1` (~900 строк) | Тестировщик стратегий: HTTP/TLS-проверки целей из `targets.txt`, DPI-чекеры, прогон всех конфигов с перезапуском winws |
| `utils/targets.txt` | Цели проверок: Discord (4), YouTube (4), Google (2), Cloudflare (2), ping-цели DNS |
| `utils/check_updates.enabled` | Флаг включённой автопроверки обновлений |
| `utils/game_filter.enabled` | (создаётся) режим и диапазоны портов Game Filter |

## 3. Модель работы

### 3.1. Запуск стратегии (general*.bat)

1. `chcp 65001`, переход в каталог скрипта.
2. Вызовы `service.bat`: `status_zapret` (предупреждение, если служба
   уже работает + включение TCP timestamps через netsh), `check_updates`
   (фоновая проверка новой версии на GitHub), `load_game_filter`
   (чтение `utils/game_filter.enabled` в переменные `GameFilterTCP/UDP`),
   `load_user_lists` (создание `*-user.txt` при отсутствии).
3. Запуск `winws.exe` (minimized) с длинной командной строкой.

### 3.2. Командная строка winws (на примере general.bat)

Общие фильтры WinDivert:
- `--wf-tcp=80,443,2053,2083,2087,2096,8443,%GameFilterTCP%`
- `--wf-udp=443,19294-19344,50000-50100,%GameFilterUDP%`

Далее профили (разделены `--new`), каждый = фильтр L3/L4/L7 + набор
параметров `--dpi-desync-*`:

1. UDP 443 (QUIC) по hostlist-general → `fake` ×6 с фейком `quic_initial_www_google_com.bin`.
2. UDP 19294-19344, 50000-50100 (Discord voice, L7=discord,stun) → `fake` ×6 с `ACTIVE_DISCORD_UDP.bin`.
3. TCP 2053,2083,2087,2096,8443 (Cloudflare-порты, discord.media) → `multisplit`, split-seqovl=681, pos=1, паттерн `tls_clienthello_www_google_com.bin`.
4. TCP 443 по list-google → `multisplit` seqovl=681 pos=1 + `--ip-id=zero`.
5. TCP 80,443 по list-general → `multisplit` seqovl=568 pos=1, паттерн `tls_clienthello_4pda_to.bin`.
6. UDP 443 по ipset-all → `fake` ×6 QUIC.
7. TCP 80,443,8443 по ipset-all → `multisplit` seqovl=568.
8. TCP GameFilter по ipset → `multisplit any-protocol cutoff=n3`.
9. UDP GameFilter по ipset → `fake` ×12, `fake-unknown-udp=ACTIVE_GAME_UDP.bin`, `cutoff=n2`.

Стратегии ALT отличаются наборами техник (`fake`, `multisplit`,
`multidisorder`, `syndata`, `hostfakesplit`, `fakedsplit`), fooling-режимами
(`ts` — TCP timestamps, `badseq` с increment 2/1000/10000000),
`fake-tls-mod` (`rnd`, `dupsid`, `sni=...`, `none`), позициями сплита
(1, 2, `sniext+1`, `midsld`), seqovl (568/652/664/679/681), repeats
(3-14) и cutoff (n2-n5). Полная матрица — в `STRATEGY_COMPATIBILITY.md`.

### 3.3. Служба Windows (service.bat → Install Service)

- Парсит выбранный .bat, извлекает аргументы winws (учитывает кавычки,
  `^`, аргументы со значениями `sni/host/altorder`).
- `sc create zapret binPath= "winws.exe <args>" start= auto`, `sc start`.
- Записывает имя стратегии в реестр:
  `HKLM\System\CurrentControlSet\Services\zapret` → `zapret-discord-youtube`.
- Remove Services: `net stop zapret`, `sc delete zapret`, kill winws.exe,
  остановка и удаление служб `WinDivert`/`WinDivert14`.

### 3.4. Переключатели состояния

- **Game Filter**: файл `utils/game_filter.enabled` (`mode=disabled|all|tcp|udp`,
  `tcp=...`, `udp=...`, диапазоны по умолчанию 1024-65535). При disabled
  подставляется фиктивный порт `12`. Требует перезапуска стратегии.
- **IPSet Filter**: состояние хранится в содержимом `lists/ipset-all.txt`
  (none/any/loaded), переключение = переименование файла и `.backup`.
- **Auto-Update Check**: наличие файла `utils/check_updates.enabled`.
- **Replace active fakes**: копирование выбранного `.bin` поверх
  `ACTIVE_DISCORD_UDP.bin` / `ACTIVE_GAME_UDP.bin` (сравнение по SHA256).

### 3.5. Диагностика (service.bat → Run Diagnostics)

Проверки: BFE (Base Filtering Engine) запущен; системный прокси (реестр
HKCU Internet Settings); TCP timestamps включены (netsh, при отсутствии —
включает); конфликтующее ПО: AdguardSvc.exe, Killer, Intel Connectivity
Network Service, Check Point (TracSrvWrapper/EPWD), SmartByte; кириллица
в пути установки; OneDrive; наличие WinDivert64.sys; VPN-службы;
Secure DNS (DoH-флаги в реестре Dnscache); hosts-файл (записи youtube);
зависший WinDivert без winws (лечит: удаление службы + поиск GoodbyeDPI);
конфликтующие службы обходов (GoodbyeDPI, discordfix_zapret, winws1/2) —
предлагает удалить; очистка кэша Discord (Stable/PTB/Canary/Development:
удаление Cache, Code Cache, GPUCache, с закрытием процесса).

### 3.6. Тестирование (utils/test zapret.ps1)

- Standard tests: HTTP/TLS-проверки целей из `targets.txt` + ping.
- DPI checkers: наборы проверок против DPI разных провайдеров
  (Cloudflare, Amazon и др.) с payload-файлами.
- Режим "All configs": последовательно запускает каждый general*.bat,
  ждёт готовности winws (до 5 с), прогоняет проверки, останавливает,
  пишет результаты в `utils/test results/`.
- Учитывает ipset-режим (умеет временно переключать в `any` с бэкапом),
  конфликт службы zapret, снапшот/восстановление процессов winws.

### 3.7. Обновления

- Проверка версии: `raw.githubusercontent.com/.../.service/version.txt`,
  сравнение с LOCAL_VERSION, открытие страницы релиза.
- Update IPSet List: скачивание `ipset-service.txt` из репозитория в
  `lists/ipset-all.txt` (curl с `--ssl-no-revoke` на libcurl/7 или
  `--ssl-revoke-best-effort`; fallback — PowerShell Invoke-WebRequest).
- Update Hosts File: скачивание `.service/hosts`, сравнение первой/последней
  строки с системным hosts, при расхождении — открытие notepad и explorer
  для ручного копирования (записей для Discord voice и веб-Telegram).

## 4. Ключевые наблюдения для MXDS

1. **Вся сетевая логика — в аргументах winws.** Перенос = реализация
   движка, принимающего эквивалентный набор параметров десинхронизации
   (фильтры WinDivert, профили L7, hostlist/ipset, техники fake/split/
   disorder/syndata, fooling ts/badseq, cutoff, repeats).
2. **Конфигурация стратегии = структурированный список профилей.**
   Bat-файлы — плоский текст; MXDS должен хранить стратегии в типизированном
   формате (JSON), с парсером оригинального синтаксиса для импорта.
3. **Состояние оригинала размазано** по файлам-флагам, содержимому
   ipset-all.txt и реестру. В MXDS — централизовано в Config Manager.
4. **Автозапуск** реализован через службу Windows (`sc create`); winws
   требует админ-права (драйвер WinDivert). MXDS потребует того же:
   либо служба, либо запуск с elevation (решение — Цикл 3).
5. **TCP timestamps** обязательны для fooling `ts` — движок должен
   проверять/включать (`netsh interface tcp set global timestamps=enabled`).
6. **Антивирусы** детектят WinDivert как RiskTool — нужен раздел в
   документации и диагностике.
7. **Пути с кириллицей/OneDrive** — известный источник проблем оригинала;
   MXDS должен диагностировать это же.
8. Лицензия MIT позволяет переиспользование списков и payload'ов с
   сохранением copyright (решение D-004). WinDivert — LGPLv3 (проверка
   условий распространения — Цикл 2).
