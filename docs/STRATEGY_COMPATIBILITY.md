# Совместимость стратегий оригинала

Дата: 2026-11-13. Источник: `divert\example\general*.bat` (24 стратегии,
v1.10.4).

Статусы по PROMPT.md (Цикл 8): SUPPORTED / PARTIALLY_SUPPORTED /
UNSUPPORTED / NOT_TESTED. На момент Цикла 1 все стратегии — NOT_TESTED:
совместимость будет подтверждаться только фактическими прогонами
Rust Engine (Циклы 7-8, 15).

## 1. Общая структура всех стратегий

Все 24 стратегии используют одинаковые WinDivert-фильтры захвата:

- `--wf-tcp=80,443,2053,2083,2087,2096,8443,%GameFilterTCP%`
- `--wf-udp=443,19294-19344,50000-50100,%GameFilterUDP%`

и 9 профилей обработки (порядок general.bat): QUIC/hostlist-general,
Discord voice UDP (L7=discord,stun), Cloudflare-порты TCP (discord.media),
Google TCP 443, hostlist-general TCP 80/443, ipset UDP 443, ipset TCP,
GameFilter TCP, GameFilter UDP. Отдельные стратегии меняют техники
внутри профилей и число профилей.

## 2. Используемые техники десинхронизации

| Техника | Параметры | Смысл |
|---|---|---|
| `fake` | `--dpi-desync-repeats=N`, `--dpi-desync-fake-{tls,quic,http,discord,stun,unknown,unknown-udp}=file/hex` | отправка фейкового пакета до настоящего |
| `multisplit` | `--dpi-desync-split-pos=N[,sniext+1,midsld]`, `--dpi-desync-split-seqovl=N`, `--dpi-desync-split-seqovl-pattern=file` | фрагментация с перекрытием sequence |
| `multidisorder` | те же split-параметры | фрагментация с нарушением порядка |
| `syndata` | — | фейковый SYN+DATA |
| `hostfakesplit` | `--dpi-desync-hostfakesplit-mod=host=...,altorder=1` | фейк + сплит у хоста |
| `fakedsplit` | `--dpi-desync-fakedsplit-pattern=0x00` | фейк-сплит с паттерном |
| fooling | `--dpi-desync-fooling=ts` / `badseq`, `--dpi-desync-badseq-increment=N` | порча фейков (timestamps / bad sequence) |
| fake-tls-mod | `--dpi-desync-fake-tls-mod=rnd,dupsid,sni=...` / `none` | модификация фейкового TLS |
| cutoff | `--dpi-desync-cutoff=nN` | прекращение десинхронизации после N пакетов |
| any-protocol | `--dpi-desync-any-protocol=1` | применять к нераспознанным протоколам |
| `--ip-id=zero` | — | обнуление IP ID |

## 3. Матрица стратегий

| Стратегия | Основные техники | Fooling | Отличительные параметры |
|---|---|---|---|
| general | multisplit (осн.), fake | — | seqovl 568/681, pos 1, ip-id=zero (google) |
| ALT | fake,fakedsplit (осн.) | ts | fakedsplit-pattern=0x00, cutoff n3/n4 |
| ALT2 | multisplit, fake | — | seqovl 652, split-pos 2 |
| ALT3 | fake,hostfakesplit | — | hostfakesplit-mod host=ya.ru/www.google.com, altorder=1; fake-tls-mod rnd,dupsid,sni |
| ALT4 | fake,multisplit | badseq | badseq-increment=1000 |
| ALT5 | fake; syndata,multidisorder (TCP-списки) | — | repeats 14 (game udp); 23 строки (нет части профилей) |
| ALT6 | multisplit, fake | — | seqovl 681, pos 1 |
| ALT7 | multisplit, fake; syndata (часть) | — | split-pos 2,sniext+1; seqovl 679 |
| ALT8 | fake (все TCP-профили) | badseq | badseq-increment=2, fake-tls-mod=none |
| ALT9 | hostfakesplit, fake | ts (+md5sig частично) | host=ozon.ru/www.google.com, repeats 4 |
| ALT10 | fake (все TCP-профили) | ts | fake-tls-mod=none, cutoff n2/n3 |
| ALT11 | fake,multisplit | ts | seqovl 664/681, repeats 8/10/11 |
| ALT12 | fake,multisplit; hostfakesplit (часть) | ts | host=www.google.com, repeats 3/8/10/11 |
| ALT13 | fake,hostfakesplit; multisplit (часть) | ts | host=mail.ru,altorder=1, repeats 5/7 |
| EXP | fake,multisplit; hostfakesplit | ts | seqovl 480/664/681, 3× any-protocol, cutoff n4 |
| FAKE TLS AUTO | fake,multidisorder | badseq | fake-tls-mod rnd,dupsid,sni=www.google.com; fake-tls=0x00000000/`!`; split-pos 1,midsld |
| FAKE TLS AUTO ALT | fake,fakedsplit | badseq | badseq-increment=2 |
| FAKE TLS AUTO ALT2 | fake,multisplit | badseq | badseq-increment=10000000, seqovl 681 |
| FAKE TLS AUTO ALT3 | fake,multisplit | ts | seqovl 681 |
| SIMPLE FAKE | fake (почти все), hostfakesplit | ts | repeats 6/12 |
| SIMPLE FAKE ALT | fake | badseq | badseq-increment=2 |
| SIMPLE FAKE ALT2 | fake | ts | cutoff n3/n5 |

## 4. Payload-файлы, используемые стратегиями

QUIC Initial: `quic_initial_www_google_com.bin` (основной), 4pda, 5ka,
lemana_pro, rutube, steam (3), tencent.
TLS ClientHello: `tls_clienthello_www_google_com.bin`, `4pda_to.bin`
(основные), awswaf, max_ru, sochi_park, steam (2), teamspeak, 5ka, sferum.
Discord/STUN: `ACTIVE_DISCORD_UDP.bin` (копия `stun.bin`/`stun2.bin`).
Game UDP: `ACTIVE_GAME_UDP.bin`.

## 5. План подтверждения совместимости

1. Цикл 7-8: Rust Engine реализует все техники раздела 2.
2. Каждая стратегия импортируется в типизированный JSON-профиль
   (парсер синтаксиса winws-аргументов).
3. Цикл 15: поведенческие прогоны — доступность целей из targets.txt
   с оригинальным winws и с MXDS на одной машине, сравнение результатов.
4. Статус стратегии обновляется только по факту прогона; до этого
   в GUI/CLI отображается NOT_TESTED.
