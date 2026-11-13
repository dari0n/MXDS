# Карта зависимостей оригинального проекта

Дата: 2026-11-13. Источник: `divert\example` (v1.10.4).

## 1. Исполняемые и бинарные компоненты

| Компонент | Источник | Роль в оригинале | Статус в MXDS |
|---|---|---|---|
| `winws.exe` (203776 B) | bol-van/zapret-win-bundle | Сетевой движок DPI-desync | ЗАПРЕЩЁН. Заменяется Rust Engine |
| `WinDivert.dll` (47616 B) | WinDivert (через zapret-win-bundle) | userspace-API драйвера | Разрешён как системная зависимость; FFI-биндинги в Цикле 2/7 |
| `WinDivert64.sys` (94144 B) | WinDivert | kernel-драйвер перехвата | Разрешён при соблюдении LGPL; проверка подписи/версии — Цикл 2 |
| `cygwin1.dll` (2954293 B) | Cygwin | рантайм для winws.exe | Не нужен (Rust MSVC) |
| `*.bin` payload'ы (26 файлов) | zapret-win-bundle + Flowseal | фейковые TLS/QUIC/STUN/UDP-пакеты | Разрешены (D-004); копируются в `assets/payloads/` |

## 2. Данные

| Данные | Формат | Назначение | MXDS |
|---|---|---|---|
| `list-general.txt` | домены, `^`=точное совпадение | основной hostlist | Копия в `lists/`, формат сохраняется |
| `list-google.txt` | домены | hostlist Google/YouTube | Аналогично |
| `list-exclude.txt` | домены | исключения | Аналогично |
| `ipset-all.txt` | IP/CIDR; пустой=any; `203.0.113.113/32`=none | ipset обхода | Данные сохраняются; режим переносится в конфиг |
| `ipset-exclude.txt` | IP/CIDR | исключения (RFC1918 и др.) | Аналогично |
| `*-user.txt` | домены/IP | пользовательские списки | Автосоздание при первом запуске |
| `utils/targets.txt` | `Key = "url"` / `PING:ip` | цели тестов | Формат-источник для Connectivity |
| `utils/check_updates.enabled` | файл-флаг | автопроверка обновлений | Поле конфигурации |
| `utils/game_filter.enabled` | ini-подобный (`mode=`, `tcp=`, `udp=`) | Game Filter | Поле конфигурации с валидацией |

## 3. Системные зависимости Windows

| Зависимость | Использование оригиналом | Критичность |
|---|---|---|
| Права администратора | загрузка драйвера WinDivert, sc.exe, netsh | Обязательно |
| Base Filtering Engine (BFE) | требуется WinDivert | Обязательно; диагностика |
| Service Control Manager | служба zapret, удаление WinDivert/WinDivert14 | Обязательно для автозапуска |
| netsh | включение TCP timestamps | Обязательно для fooling=ts |
| Реестр HKLM\...\Services\zapret | хранение имени активной стратегии | Переносится в конфиг MXDS |
| Реестр HKCU Internet Settings | проверка системного прокси | Диагностика |
| Реестр Dnscache DohFlags | проверка Secure DNS | Диагностика |
| PowerShell 3.0+ | меню, тесты, скачивания, вывод цвета | Не нужен в MXDS |
| curl.exe (System32) | обновления ipset/hosts | Не нужен (reqwest/rustls) |

## 4. Сетевые зависимости (исходящие)

| URL | Назначение | MXDS |
|---|---|---|
| `raw.githubusercontent.com/Flowseal/zapret-discord-youtube/main/.service/version.txt` | проверка версии | Опционально, по конфигу |
| `.../ipset-service.txt` | обновление ipset-all | Опционально |
| `.../.service/hosts` | обновление hosts | Опционально (см. F-10) |
| Цели из targets.txt | тесты доступности | Connectivity (Цикл 11) |

## 5. Известные конфликты (из диагностики оригинала)

Adguard (AdguardSvc.exe), Killer, Intel Connectivity Network Service,
Check Point (TracSrvWrapper, EPWD), SmartByte, GoodbyeDPI,
discordfix_zapret, winws1, winws2, VPN-службы, антивирусные детекты
WinDivert (RiskTool.Multi.WinDivert).

Все переносятся в Diagnostics MXDS (Цикл 10).

## 6. Зависимости MXDS (план, Цикл 4)

Rust stable 1.100+, MSVC x64. Ожидаемые crate-направления (уточняются
в Цикле 3): tokio, tracing, tracing-subscriber, serde/serde_json, clap,
thiserror, eframe/egui, windows-sys (WinAPI), reqwest или ureq+rustls
(проверки доступности). WinDivert — через собственный FFI-слой
(`src/windivert/ffi.rs`), unsafe минимизируется и документируется.
