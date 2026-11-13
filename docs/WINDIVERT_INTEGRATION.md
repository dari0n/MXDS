# WinDivert Integration

Дата: 2026-11-13. Статус: аналитический документ Цикла 2.
Источники: официальная документация WinDivert 2.2 (reqrypt.org/windivert-doc.html),
факты о локальных бинарниках из `divert\example\bin`.

## 1. Что такое WinDivert

WinDivert — user-mode пакет перехвата/модификации/инъекции трафика для
Windows 10/11 и Windows Server: WDF/WFP callout-драйвер (kernel) +
user-mode DLL с простым API. Позволяет реализовать всю логику движка MXDS
в user-mode Rust без написания kernel-кода.

Лицензия: dual license — LGPL v3 ИЛИ GPL v2 (на выбор). MXDS использует
LGPL v3: динамическая загрузка неизменённого WinDivert.dll, без
статической линковки и без модификации исходников WinDivert.

## 2. Факты о поставляемых бинарниках (проверено 2026-11-13)

| Файл | Размер | Версия | Подпись |
|---|---|---|---|
| `WinDivert.dll` | 47 616 B | нет version info | НЕ подписана |
| `WinDivert64.sys` | 94 144 B | 2.2 | Valid ( Authenticode, переподписанная сборка из zapret-win-bundle ) |

Выводы:
- Драйвер версии 2.2, подпись валидна — загружается на Windows 10/11 x64
  (иначе `ERROR_INVALID_IMAGE_HASH` 577).
- DLL без подписи — обычная практика для WinDivert; детектируется
  некоторыми антивирусами как RiskTool (см. SECURITY_REVIEW.md).
- MXDS распространяет эти файлы как отдельные нетронутые бинарники
  с указанием происхождения (basil00/Divert) и текста LGPL v3.

## 3. Используемое подмножество API

Движку MXDS нужен только слой `WINDIVERT_LAYER_NETWORK` (пакеты к/от
локальной машины; block+inject+data). Слои FLOW/SOCKET/REFLECT —
опционально для будущей диагностики, не входят в MVP.

Функции (FFI-граница `src/windivert/ffi.rs`):

| Функция | Назначение в MXDS |
|---|---|
| `WinDivertOpen(filter, layer, priority, flags)` | открытие хендла; авто-установка драйвера; требует админ |
| `WinDivertRecv` / `WinDivertRecvEx` | приём пакетов (Ex — batched + overlapped для производительности) |
| `WinDivertSend` / `WinDivertSendEx` | инъекция оригинальных и фейковых пакетов |
| `WinDivertShutdown` | управляемая остановка приёма (для graceful stop) |
| `WinDivertClose` | освобождение хендла |
| `WinDivertSetParam` / `WinDivertGetParam` | queue length/time/size; VERSION_MAJOR/MINOR — проверка совместимости |
| `WinDivertHelperParsePacket` | разбор IP/IPv6/TCP/UDP заголовков |
| `WinDivertHelperCalcChecksums` | пересчёт checksums после модификации |
| `WinDivertHelperDecrementTTL` | для impostor-инъекций |

Структуры FFI: `WINDIVERT_ADDRESS` (Timestamp, Layer, Event, флаги
Sniffed/Outbound/Loopback/Impostor/IPv6/*Checksum, union Network/Flow/
Socket/Reflect), `WINDIVERT_IPHDR`, `WINDIVERT_IPV6HDR`,
`WINDIVERT_TCPHDR`, `WINDIVERT_UDPHDR`, `WINDIVERT_ICMPHDR`,
`WINDIVERT_ICMPV6HDR`. Точные C-layout'ы будут воспроизведены в
`ffi.rs` с `#[repr(C)]` и проверками размеров/выравнивания
(compile-time asserts).

## 4. Параметры open, используемые стратегиями

Оригинал открывает хендл с фильтрами `--wf-tcp=...` / `--wf-udp=...`
на языке фильтров WinDivert (например `tcp.DstPort==80 or ...`).
MXDS: генерация строки фильтра из типизированной конфигурации движка
(порты TCP/UDP + Game Filter), `priority = 0`, без SNIFF/DROP-флагов.

Параметры очереди (SetParam): дефолтные значения WinDivert достаточны;
тюнинг `QUEUE_LENGTH/QUEUE_TIME/QUEUE_SIZE` — через конфиг с валидацией
диапазонов (MIN/MAX из документации).

**Уточнение Цикла 7 (живой прогон 2026-11-13):** поставляемая сборка
WinDivert (zapret-win-bundle) принимает `QUEUE_SIZE` в **байтах**
(65536..=8388608), а не в КБ, как в документации 2.2. Значения в КБ
возвращают `ERROR_INVALID_PARAMETER` (87). Реализация: `QueueSizeBytes`
в adapter.rs, конфиг хранит КБ и умножает на 1024.

## 5. Жизненный цикл

1. Проверка: файлы WinDivert.dll/WinDivert64.sys рядом с MXDS
   (диагностика; отсутствие → `ERROR_FILE_NOT_FOUND`).
2. `WinDivertOpen` → драйвер устанавливается автоматически (служба
   `WinDivert`). Требуются права администратора.
3. Рабочий цикл: Recv(Ex) → обработка по профилям → Send(Ex).
4. Остановка: `WinDivertShutdown(RECV)` → дочитать очередь →
   `WinDivertClose`. Драйвер остаётся загруженным до перезагрузки или
   `sc stop/delete WinDivert` (делается при явном удалении/деинсталляции,
   как в оригинале — Remove Services).
5. Каждый хендл владения оборачивается в Rust-тип с `Drop`, гарантирующим
   Close при панике/ошибке.

## 6. Ошибки WinDivertOpen и их обработка в MXDS

| Код | Имя | Обработка MXDS |
|---|---|---|
| 2 | ERROR_FILE_NOT_FOUND | FAIL-диагностика: нет dll/sys рядом с exe |
| 5 | ERROR_ACCESS_DENIED | FAIL: нет прав администратора → рекомендация перезапуска с elevation |
| 87 | ERROR_INVALID_PARAMETER | FAIL: ошибка генерации фильтра (баг MXDS) → лог + аварийная остановка движка |
| 577 | ERROR_INVALID_IMAGE_HASH | FAIL: подпись драйвера не принята → замена бинарников |
| 654 | ERROR_DRIVER_FAILED_PRIOR_UNLOAD | FAIL: загружена несовместимая версия драйвера → перезагрузка ОС |
| 1060 | ERROR_SERVICE_DOES_NOT_EXIST | только с флагом NO_INSTALL (MXDS не использует) |
| 1275 | ERROR_DRIVER_BLOCKED | FAIL: антивирус/политика блокирует драйвер → рекомендация по исключениям |
| 1753 | EPT_S_NOT_REGISTERED | FAIL: остановлена служба BFE → рекомендация запуска BFE |

## 7. Известные ограничения

- Impostor-пакеты могут рециркулировать (Impostor=1): движок обязан
  фильтровать свои инъекции (по Impostor-флагу / TTL-стратегии).
- Loopback считается только outbound; loopback-захват стратегиям не нужен.
- Checksum offload: у захваченных пакетов checksum-флаги могут быть сброшены
  — движок пересчитывает checksums через HelperCalcChecksums после любой
  модификации.
- Пакеты не квитаются бесконечно: медленная обработка = потери →
  мониторинг переполнения очереди в диагностике движка.
- WinDivert не предоставляет PID на NETWORK-слое — фильтрация «по
  процессу» невозможна (не требуется стратегиями).

## 8. Стратегия unsafe

- Весь unsafe сосредоточен в `src/windivert/ffi.rs` (объявления extern,
  вызовы) и тонком safe-wrapper `src/windivert/adapter.rs`.
- Каждый unsafe-блок: комментарий-обоснование + проверка инвариантов
  (ненулевые указатели, длины буферов, владение хендлом).
- Загрузка DLL: статическая линковка через `WinDivert.lib` невозможна без
  .lib; используем `libloading` (явный путь к DLL рядом с exe,
  проверка SHA256 перед загрузкой — см. SECURITY_REVIEW.md).

## 9. Совместимость

- Целевая версия: WinDivert 2.2 (проверяется через
  `WINDIVERT_PARAM_VERSION_MAJOR/MINOR` при старте).
- Windows 10/11 x64. Хост разработки: Windows 11 (build 26300) — подтверждено.
- 32-бит не поддерживается (целевая платформа x64 по PROMPT.md).
