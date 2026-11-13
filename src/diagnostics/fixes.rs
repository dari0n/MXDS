//! Авто-исправления диагностики (кнопка «Исправить» в GUI).
//!
//! Каждое исправление меняет настройки ОС и требует прав администратора —
//! `run_fix` проверяет elevation в первую очередь. Исправления best-effort:
//! при частичном сбое возвращают честное сообщение, что удалось, а что нет.

use crate::diagnostics::FixId;
use crate::windows::{components, integration, permissions, WindowsError};

/// DoH-шаблоны известных резолверов (Cloudflare, Google, Quad9).
/// Регистрируются через Set-DnsClientDohServerAddress — после этого Windows
/// автоматически поднимает DoH, если DNS-сервер адаптера совпадает с одним
/// из этих адресов.
const DOH_SERVERS: [(&str, &str); 5] = [
    ("1.1.1.1", "https://cloudflare-dns.com/dns-query"),
    ("1.0.0.1", "https://cloudflare-dns.com/dns-query"),
    ("8.8.8.8", "https://dns.google/dns-query"),
    ("8.8.4.4", "https://dns.google/dns-query"),
    ("9.9.9.9", "https://dns.quad9.net/dns-query"),
];

/// Выполнить авто-исправление. Возвращает человекочитаемый итог.
pub fn run_fix(id: FixId) -> Result<String, WindowsError> {
    permissions::require_elevated()?;
    match id {
        FixId::EnableTcpTimestamps => {
            integration::enable_tcp_timestamps()?;
            Ok(
                "TCP timestamps включены (netsh interface tcp set global timestamps=enabled)"
                    .into(),
            )
        }
        FixId::EnableSecureDns => configure_secure_dns(),
        FixId::CleanupWinDivert => {
            components::windivert_cleanup()?;
            Ok("остаточный драйвер WinDivert снят (службы WinDivert/WinDivert14 удалены)".into())
        }
        FixId::RemoveConflictingServices => remove_conflicting_services(),
    }
}

/// Настройка Secure DNS: регистрация DoH-шаблонов известных резолверов
/// и включение DoH на активных адаптерах:
/// - адаптер уже использует известный DoH-резолвер → включаем DoH для него
///   (DohFlags=1 в DohInterfaceSettings, как делают Параметры Windows 11);
/// - адаптер на DHCP-DNS → переключаем на Cloudflare (1.1.1.1/1.0.0.1)
///   и включаем DoH;
/// - адаптер со статическим DNS не трогаем — сообщаем в итоге.
fn configure_secure_dns() -> Result<String, WindowsError> {
    let mut script = String::from("$ErrorActionPreference = 'Stop'\n");
    for (ip, template) in DOH_SERVERS {
        script.push_str(&format!(
            "Set-DnsClientDohServerAddress -ServerAddress '{ip}' -DohTemplate '{template}' \
             -AllowFallbackToUdp $true -AutoUpgrade $true\n"
        ));
    }
    let known_ips: Vec<&str> = DOH_SERVERS.iter().map(|(ip, _)| *ip).collect();
    script.push_str(&format!(
        r#"
$known = @('{}')
$base = 'HKLM:\SYSTEM\CurrentControlSet\Services\Dnscache\InterfaceSpecificParameters'
$report = @()
$adapters = Get-NetAdapter | Where-Object {{ $_.Status -eq 'Up' -and -not $_.Virtual }}
foreach ($a in $adapters) {{
  $dns = (Get-DnsClientServerAddress -InterfaceIndex $a.ifIndex -AddressFamily IPv4).ServerAddresses
  $servers = @($dns | Where-Object {{ $known -contains $_ }})
  if ($servers.Count -eq 0) {{
    $dhcp = (Get-NetIPInterface -InterfaceIndex $a.ifIndex -AddressFamily IPv4).Dhcp -eq 'Enabled'
    if (-not $dhcp) {{ $report += "skip-static:$($a.Name)"; continue }}
    Set-DnsClientServerAddress -InterfaceIndex $a.ifIndex -ServerAddresses @('1.1.1.1','1.0.0.1')
    $servers = @('1.1.1.1','1.0.0.1')
  }}
  foreach ($s in $servers) {{
    $p = Join-Path $base "$($a.InterfaceGuid)\DohInterfaceSettings\$s"
    New-Item -Path $p -Force | Out-Null
    New-ItemProperty -Path $p -Name DohFlags -PropertyType DWord -Value 1 -Force | Out-Null
  }}
  $report += "doh-on:$($a.Name)=$($servers -join ',')"
}}
Restart-Service dnscache -Force -ErrorAction SilentlyContinue
$report -join '; '
"#,
        known_ips.join("','")
    ));

    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| WindowsError::Ipc(format!("powershell не запустился: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(WindowsError::Ipc(format!(
            "настройка Secure DNS не удалась: {}",
            stderr.trim()
        )));
    }
    let report = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Контроль результата: проверка должна видеть DoH-флаги.
    if integration::secure_dns_configured() {
        let mut msg = format!("Secure DNS включён (DoH с откатом на UDP). {report}");
        if report.contains("skip-static") {
            msg.push_str(" Адаптеры со статическим DNS не изменялись.");
        }
        Ok(msg)
    } else {
        Err(WindowsError::Ipc(format!(
            "DoH-шаблоны зарегистрированы, но активных адаптеров с DoH нет ({report}); \
             укажите DNS 1.1.1.1 в настройках адаптера"
        )))
    }
}

/// Остановка и удаление конфликтующих bypass-служб (GoodbyeDPI, zapret, ...).
fn remove_conflicting_services() -> Result<String, WindowsError> {
    let found = components::conflicting_services();
    if found.is_empty() {
        return Ok("конфликтующие службы не найдены — исправление не требуется".into());
    }
    let mut failed: Vec<String> = Vec::new();
    for name in &found {
        if let Err(e) = components::stop_and_delete(name) {
            failed.push(format!("{name}: {e}"));
        }
    }
    if failed.is_empty() {
        Ok(format!(
            "остановлены и удалены конфликтующие службы: {}",
            found.join(", ")
        ))
    } else {
        Err(WindowsError::Ipc(format!(
            "не удалось удалить часть служб: {}",
            failed.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doh_servers_well_formed() {
        for (ip, template) in DOH_SERVERS {
            assert!(ip.parse::<std::net::Ipv4Addr>().is_ok(), "{ip}");
            assert!(template.starts_with("https://"), "{template}");
        }
    }

    #[test]
    fn fix_ids_have_handlers() {
        // Каждый вариант FixId покрыт match в run_fix — компилятор следит,
        // здесь фиксируем контракт сообщений об ошибке без прав.
        for id in [
            FixId::EnableTcpTimestamps,
            FixId::EnableSecureDns,
            FixId::CleanupWinDivert,
            FixId::RemoveConflictingServices,
        ] {
            let r = run_fix(id);
            if !crate::windows::permissions::is_elevated() {
                assert!(matches!(r, Err(WindowsError::NotElevated)), "{id:?}");
            }
        }
    }
}
