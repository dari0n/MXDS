//! Каскадные проверки доступности DNS→TCP→TLS→HTTP и ICMP ping.
//! См. docs/MODULE_CONTRACTS.md п.9, PROMPT.md Цикл 11.
//!
//! Принципы:
//! - провал этапа фиксируется, вышестоящие этапы — Blocked;
//! - успех DNS не означает доступность сайта, успех TCP — рабочий HTTP;
//! - доступность внешних ресурсов не гарантируется: статусы отражают
//!   факт на момент проверки.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::diagnostics::CheckStatus;

use super::results::{ConnectivityResult, Stage, Target};
use super::ConnectivityError;

/// Тайм-аут одного этапа по умолчанию.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Путь к списку целей относительно корня установки.
pub const TARGETS_REL_PATH: &str = "config/targets.txt";

/// Встроенный список целей (fallback, если файл отсутствует/пуст).
const EMBEDDED_TARGETS: &str = include_str!("../../config/targets.txt");

// --- Парсинг целей ---

/// Разбор targets.txt (формат utils/targets.txt оригинала):
/// `Key = "https://host[/path]"` или `Key = "PING:1.2.3.4"`;
/// строки `#` и `###` — комментарии.
pub fn parse_targets(content: &str) -> Vec<Target> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().to_string();
        let value = value.trim().trim_matches('"');
        if let Some(ip) = value.strip_prefix("PING:") {
            if let Ok(addr) = ip.trim().parse::<IpAddr>() {
                out.push(Target::Ping { name, addr });
            } else {
                tracing::warn!(target = %name, value = %ip, "цель PING пропущена: невалидный IP");
            }
        } else if value.starts_with("http://") || value.starts_with("https://") {
            out.push(Target::Url {
                name,
                url: value.to_string(),
            });
        } else {
            tracing::warn!(target = %name, value, "цель пропущена: неизвестный формат");
        }
    }
    out
}

/// Загрузить цели из `<root>/config/targets.txt`; при отсутствии или
/// пустом списке — встроенный набор.
pub fn load_targets(root: &Path) -> Vec<Target> {
    let path = root.join(TARGETS_REL_PATH);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let targets = parse_targets(&content);
            if targets.is_empty() {
                tracing::warn!(path = %path.display(), "список целей пуст — встроенный набор");
                parse_targets(EMBEDDED_TARGETS)
            } else {
                targets
            }
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "targets.txt не прочитан — встроенный набор");
            parse_targets(EMBEDDED_TARGETS)
        }
    }
}

// --- URL ---

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUrl {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

/// Минимальный парсер http/https URL (цели targets.txt — без query,
/// userinfo и фрагментов).
fn parse_url(url: &str) -> Result<ParsedUrl, ConnectivityError> {
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(ConnectivityError::Http { status: 0 });
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => {
            let port: u16 = p
                .parse()
                .map_err(|_| ConnectivityError::Http { status: 0 })?;
            (h.to_string(), port)
        }
        None => (authority.to_string(), if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(ConnectivityError::Http { status: 0 });
    }
    let host = host.to_string();
    Ok(ParsedUrl {
        https,
        host,
        port,
        path: path.to_string(),
    })
}

/// Разбор статус-строки HTTP-ответа: "HTTP/1.1 200 OK" → 200.
fn parse_http_status(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

// --- Проверки ---

/// Проверить цели параллельно. Порядок результатов соответствует
/// порядку целей. Тайм-аут — на каждый этап отдельно.
pub async fn check(targets: &[Target], timeout: Duration) -> Vec<ConnectivityResult> {
    let mut set = tokio::task::JoinSet::new();
    for (idx, target) in targets.iter().enumerate() {
        let target = target.clone();
        set.spawn(async move { (idx, check_target(&target, timeout).await) });
    }
    let mut indexed = Vec::with_capacity(targets.len());
    while let Some(res) = set.join_next().await {
        match res {
            Ok(pair) => indexed.push(pair),
            Err(e) => {
                tracing::error!(error = %e, "задача проверки цели завершилась паникой");
            }
        }
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    indexed
        .into_iter()
        .flat_map(|(_, results)| results)
        .collect()
}

async fn check_target(target: &Target, timeout: Duration) -> Vec<ConnectivityResult> {
    match target {
        Target::Ping { name, addr } => vec![check_ping(name, *addr, timeout).await],
        Target::Url { name, url } => check_url(name, url, timeout).await,
    }
}

fn res(
    target: &str,
    stage: Stage,
    status: CheckStatus,
    elapsed: Duration,
    detail: impl Into<String>,
) -> ConnectivityResult {
    ConnectivityResult {
        target: target.to_string(),
        stage,
        status,
        elapsed,
        detail: detail.into(),
    }
}

/// Результаты каскада при провале этапа `failed`: предыдущие этапы уже
/// добавлены, провальный — Fail, остальные — Blocked.
fn fail_cascade(
    name: &str,
    https: bool,
    failed: Stage,
    elapsed: Duration,
    detail: String,
    mut results: Vec<ConnectivityResult>,
) -> Vec<ConnectivityResult> {
    results.push(res(name, failed, CheckStatus::Fail, elapsed, detail));
    let remaining: &[Stage] = match failed {
        Stage::Dns => {
            if https {
                &[Stage::Tcp, Stage::Tls, Stage::Http]
            } else {
                &[Stage::Tcp, Stage::Http]
            }
        }
        Stage::Tcp => {
            if https {
                &[Stage::Tls, Stage::Http]
            } else {
                &[Stage::Http]
            }
        }
        Stage::Tls => &[Stage::Http],
        _ => &[],
    };
    for stage in remaining {
        results.push(res(
            name,
            *stage,
            CheckStatus::Blocked,
            Duration::ZERO,
            "этап недостижим: провал нижестоящего",
        ));
    }
    results
}

async fn check_url(name: &str, url: &str, timeout: Duration) -> Vec<ConnectivityResult> {
    let parsed = match parse_url(url) {
        Ok(p) => p,
        Err(e) => {
            return vec![res(
                name,
                Stage::Http,
                CheckStatus::Fail,
                Duration::ZERO,
                format!("невалидный URL: {e}"),
            )];
        }
    };

    // DNS
    let started = Instant::now();
    let host_port = (parsed.host.clone(), parsed.port);
    let lookup = tokio::time::timeout(timeout, tokio::net::lookup_host(host_port)).await;
    let addrs: Vec<std::net::SocketAddr> = match lookup {
        Ok(Ok(iter)) => {
            let v: Vec<_> = iter.collect();
            if v.is_empty() {
                return fail_cascade(
                    name,
                    parsed.https,
                    Stage::Dns,
                    started.elapsed(),
                    "DNS вернул пустой список адресов".into(),
                    Vec::new(),
                );
            }
            v
        }
        Ok(Err(e)) => {
            return fail_cascade(
                name,
                parsed.https,
                Stage::Dns,
                started.elapsed(),
                format!("DNS: {e}"),
                Vec::new(),
            );
        }
        Err(_) => {
            return fail_cascade(
                name,
                parsed.https,
                Stage::Dns,
                started.elapsed(),
                format!("DNS: тайм-аут {} мс", timeout.as_millis()),
                Vec::new(),
            );
        }
    };
    let dns_elapsed = started.elapsed();
    let mut results = vec![res(
        name,
        Stage::Dns,
        CheckStatus::Pass,
        dns_elapsed,
        format!("{} → {} адрес(а)", parsed.host, addrs.len()),
    )];

    // TCP
    let started = Instant::now();
    let mut stream_opt = None;
    let mut last_err = String::new();
    for addr in addrs.iter().take(2) {
        match tokio::time::timeout(timeout, TcpStream::connect(*addr)).await {
            Ok(Ok(s)) => {
                stream_opt = Some(s);
                break;
            }
            Ok(Err(e)) => last_err = format!("TCP {addr}: {e}"),
            Err(_) => last_err = format!("TCP {addr}: тайм-аут {} мс", timeout.as_millis()),
        }
    }
    let Some(stream) = stream_opt else {
        return fail_cascade(
            name,
            parsed.https,
            Stage::Tcp,
            started.elapsed(),
            last_err,
            results,
        );
    };
    results.push(res(
        name,
        Stage::Tcp,
        CheckStatus::Pass,
        started.elapsed(),
        format!("порт {} открыт", parsed.port),
    ));

    // HTTP (и TLS для https)
    if parsed.https {
        let started = Instant::now();
        let connector = tls_connector();
        let server_name = match rustls::pki_types::ServerName::try_from(parsed.host.clone()) {
            Ok(n) => n,
            Err(e) => {
                return fail_cascade(
                    name,
                    true,
                    Stage::Tls,
                    started.elapsed(),
                    format!("невалидное имя сервера: {e}"),
                    results,
                );
            }
        };
        match tokio::time::timeout(timeout, connector.connect(server_name, stream)).await {
            Ok(Ok(tls)) => {
                results.push(res(
                    name,
                    Stage::Tls,
                    CheckStatus::Pass,
                    started.elapsed(),
                    "TLS handshake успешен".to_string(),
                ));
                http_stage(name, parsed, timeout, results, tls).await
            }
            Ok(Err(e)) => fail_cascade(
                name,
                true,
                Stage::Tls,
                started.elapsed(),
                format!("TLS: {e}"),
                results,
            ),
            Err(_) => fail_cascade(
                name,
                true,
                Stage::Tls,
                started.elapsed(),
                format!("TLS: тайм-аут {} мс", timeout.as_millis()),
                results,
            ),
        }
    } else {
        http_stage(name, parsed, timeout, results, stream).await
    }
}

/// HTTP-этап поверх установленного потока (plain или TLS).
async fn http_stage<S>(
    name: &str,
    parsed: ParsedUrl,
    timeout: Duration,
    mut results: Vec<ConnectivityResult>,
    mut stream: S,
) -> Vec<ConnectivityResult>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let started = Instant::now();
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: MXDS-Connectivity/0.1\r\n\
         Accept: */*\r\nConnection: close\r\n\r\n",
        parsed.path, parsed.host
    );
    let io = async {
        stream.write_all(request.as_bytes()).await?;
        // Достаточно статус-строки; читаем до первого \n (лимит 8 КиБ).
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 512];
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(1).any(|w| w == b"\n") || buf.len() > 8192 {
                break;
            }
        }
        std::io::Result::Ok(buf)
    };
    match tokio::time::timeout(timeout, io).await {
        Ok(Ok(buf)) => {
            let line = String::from_utf8_lossy(&buf);
            let first = line.lines().next().unwrap_or("").trim().to_string();
            match parse_http_status(&first) {
                Some(status) => results.push(res(
                    name,
                    Stage::Http,
                    CheckStatus::Pass,
                    started.elapsed(),
                    format!("HTTP {status}"),
                )),
                None => results.push(res(
                    name,
                    Stage::Http,
                    CheckStatus::Fail,
                    started.elapsed(),
                    format!(
                        "не-HTTP ответ: {}",
                        first.chars().take(80).collect::<String>()
                    ),
                )),
            }
        }
        Ok(Err(e)) => results.push(res(
            name,
            Stage::Http,
            CheckStatus::Fail,
            started.elapsed(),
            format!("HTTP: {e}"),
        )),
        Err(_) => results.push(res(
            name,
            Stage::Http,
            CheckStatus::Fail,
            started.elapsed(),
            format!("HTTP: тайм-аут {} мс", timeout.as_millis()),
        )),
    }
    results
}

fn tls_connector() -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

// --- Ping (ICMP) ---

#[cfg(windows)]
async fn check_ping(name: &str, addr: IpAddr, timeout: Duration) -> ConnectivityResult {
    let name_owned = name.to_string();
    let name_err = name_owned.clone();
    tokio::task::spawn_blocking(move || ping_blocking(&name_owned, addr, timeout))
        .await
        .unwrap_or_else(|e| {
            res(
                &name_err,
                Stage::Ping,
                CheckStatus::Fail,
                Duration::ZERO,
                format!("задача ping прервана: {e}"),
            )
        })
}

#[cfg(windows)]
fn ping_blocking(name: &str, addr: IpAddr, timeout: Duration) -> ConnectivityResult {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY,
    };

    let IpAddr::V4(v4) = addr else {
        return res(
            name,
            Stage::Ping,
            CheckStatus::Fail,
            Duration::ZERO,
            "IPv6 ping не поддержан (цели targets.txt — IPv4)",
        );
    };

    let started = Instant::now();
    // SAFETY: IcmpCreateFile не имеет параметров; хендл закрывается
    // IcmpCloseHandle ровно один раз (RAII ниже).
    let handle = unsafe { IcmpCreateFile() };
    if handle.is_null() {
        return res(
            name,
            Stage::Ping,
            CheckStatus::Fail,
            started.elapsed(),
            "IcmpCreateFile failed",
        );
    }
    struct IcmpHandle(*mut core::ffi::c_void);
    impl Drop for IcmpHandle {
        fn drop(&mut self) {
            // SAFETY: хендл открыт IcmpCreateFile, закрываем один раз.
            unsafe { IcmpCloseHandle(self.0) };
        }
    }
    let handle = IcmpHandle(handle);

    let payload = [0xA5u8; 32];
    let dest = u32::from_ne_bytes(v4.octets());
    // Буфер ответа: ICMP_ECHO_REPLY + payload + 8 (запас по MSDN).
    let reply_size = (std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len() + 8) as u32;
    let mut reply = vec![0u8; reply_size as usize];
    // SAFETY: handle валиден; reply — буфер reply_size байт; payload
    // жив на время вызова; опции не задаём (null).
    let count = unsafe {
        IcmpSendEcho(
            handle.0,
            dest,
            payload.as_ptr() as *const core::ffi::c_void,
            payload.len() as u16,
            std::ptr::null(),
            reply.as_mut_ptr() as *mut core::ffi::c_void,
            reply_size,
            timeout.as_millis() as u32,
        )
    };
    let elapsed = started.elapsed();
    if count == 0 {
        return res(
            name,
            Stage::Ping,
            CheckStatus::Fail,
            elapsed,
            format!("нет ответа (тайм-аут {} мс)", timeout.as_millis()),
        );
    }
    // SAFETY: count > 0 — как минимум одна ICMP_ECHO_REPLY записана
    // в начало буфера.
    let reply = unsafe { &*(reply.as_ptr() as *const ICMP_ECHO_REPLY) };
    if reply.Status == 0 {
        // IP_SUCCESS
        res(
            name,
            Stage::Ping,
            CheckStatus::Pass,
            elapsed,
            format!("ответ за {} мс", reply.RoundTripTime),
        )
    } else {
        res(
            name,
            Stage::Ping,
            CheckStatus::Fail,
            elapsed,
            format!("ICMP status {}", reply.Status),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_targets_file_format() {
        let content = r#"
# комментарий
### Секция
DiscordMain   = "https://discord.com"
GoogleDNS     = "PING:8.8.8.8"
Broken        = "PING:999.1.1.1"
Unknown       = "ftp://example.com"
NoEquals line
HttpTarget    = "http://127.0.0.1:8080/path"
"#;
        let targets = parse_targets(content);
        assert_eq!(targets.len(), 3);
        assert!(
            matches!(&targets[0], Target::Url { name, url } if name == "DiscordMain" && url == "https://discord.com")
        );
        assert!(
            matches!(&targets[1], Target::Ping { name, addr } if name == "GoogleDNS" && *addr == "8.8.8.8".parse::<IpAddr>().unwrap())
        );
        assert!(matches!(&targets[2], Target::Url { name, .. } if name == "HttpTarget"));
    }

    #[test]
    fn embedded_targets_parse() {
        let targets = parse_targets(EMBEDDED_TARGETS);
        // 12 URL + 4 PING из оригинального targets.txt.
        assert_eq!(targets.len(), 16, "встроенный список целей полон");
    }

    #[test]
    fn parse_url_variants() {
        let u = parse_url("https://discord.com").unwrap();
        assert_eq!(
            u,
            ParsedUrl {
                https: true,
                host: "discord.com".into(),
                port: 443,
                path: "/".into()
            }
        );
        let u = parse_url("http://127.0.0.1:8080/x").unwrap();
        assert_eq!(
            u,
            ParsedUrl {
                https: false,
                host: "127.0.0.1".into(),
                port: 8080,
                path: "/x".into()
            }
        );
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("https://").is_err());
        assert!(parse_url("https://host:notaport").is_err());
    }

    #[test]
    fn parse_http_status_line() {
        assert_eq!(parse_http_status("HTTP/1.1 200 OK"), Some(200));
        assert_eq!(parse_http_status("HTTP/2 404"), Some(404));
        assert_eq!(parse_http_status("garbage"), None);
    }

    #[tokio::test]
    async fn cascade_against_local_http_server() {
        // Локальный HTTP-сервер: DNS (127.0.0.1), TCP, HTTP — Pass;
        // TLS не применим (http), стадия отсутствует.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let targets = vec![Target::Url {
            name: "local".into(),
            url: format!("http://127.0.0.1:{port}/"),
        }];
        let results = check(&targets, Duration::from_secs(2)).await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].stage, Stage::Dns);
        assert_eq!(results[0].status, CheckStatus::Pass);
        assert_eq!(results[1].stage, Stage::Tcp);
        assert_eq!(results[1].status, CheckStatus::Pass);
        assert_eq!(results[2].stage, Stage::Http);
        assert_eq!(results[2].status, CheckStatus::Pass);
        assert_eq!(results[2].detail, "HTTP 200");
    }

    #[tokio::test]
    async fn cascade_tcp_failure_blocks_upper() {
        // Порт 1 на loopback закрыт: TCP Fail, HTTP Blocked.
        let targets = vec![Target::Url {
            name: "closed".into(),
            url: "http://127.0.0.1:1/".into(),
        }];
        let results = check(&targets, Duration::from_millis(500)).await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].status, CheckStatus::Pass); // DNS
        assert_eq!(results[1].stage, Stage::Tcp);
        assert_eq!(results[1].status, CheckStatus::Fail);
        assert_eq!(results[2].stage, Stage::Http);
        assert_eq!(results[2].status, CheckStatus::Blocked);
    }

    #[tokio::test]
    async fn cascade_dns_failure_blocks_all() {
        let targets = vec![Target::Url {
            name: "nodns".into(),
            url: "https://nonexistent.invalid/".into(),
        }];
        let results = check(&targets, Duration::from_secs(2)).await;
        assert_eq!(results[0].stage, Stage::Dns);
        // .invalid обязан не резолвиться; если резолвер провайдера
        // перехватывает NXDOMAIN — тест средепендентен, тогда пропускаем.
        if results[0].status == CheckStatus::Fail {
            assert_eq!(results.len(), 4);
            assert!(results[1..]
                .iter()
                .all(|r| r.status == CheckStatus::Blocked));
        }
    }

    #[tokio::test]
    #[ignore = "требует сеть: ping 1.1.1.1"]
    async fn ping_cloudflare_dns() {
        let targets = vec![Target::Ping {
            name: "cf".into(),
            addr: "1.1.1.1".parse().unwrap(),
        }];
        let results = check(&targets, Duration::from_secs(3)).await;
        assert_eq!(results[0].status, CheckStatus::Pass);
    }
}
