//! Проверки системы: версия ОС, BFE, TCP timestamps, прокси, DoH, пути.
//! См. docs/WINDOWS_INTEGRATION.md п.1, п.4, п.5.

use std::path::Path;

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD, REG_SZ,
};
use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

use crate::windows::{wide, WindowsError};

/// Версия ОС.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsVersion {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub ubr: u32,
}

/// Версия ОС через RtlGetVersion (истинная, без compatibility-шимов)
/// + UBR из реестра.
pub fn os_version() -> Result<OsVersion, WindowsError> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: info — валидный буфер размера OSVERSIONINFOW, размер
    // записан в dwOSVersionInfoSize, как требует API.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status != 0 {
        return Err(WindowsError::Unknown(status as u32));
    }
    let ubr = reg_read_dword(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "UBR",
    )
    .unwrap_or(0);
    Ok(OsVersion {
        major: info.dwMajorVersion,
        minor: info.dwMinorVersion,
        build: info.dwBuildNumber,
        ubr,
    })
}

/// Целевая платформа: Windows 10/11 x64 (build 17763+).
pub fn is_supported_os() -> Result<bool, WindowsError> {
    let v = os_version()?;
    Ok(v.major == 10 && v.build >= 17763)
}

/// Информация о системном прокси (HKCU Internet Settings).
#[derive(Debug, Clone)]
pub struct ProxyInfo {
    pub server: String,
}

/// Системный прокси включён? Только чтение реестра, без изменений.
pub fn system_proxy() -> Option<ProxyInfo> {
    let enabled = reg_read_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        "ProxyEnable",
    )?;
    if enabled == 0 {
        return None;
    }
    let server = reg_read_string(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
        "ProxyServer",
    )
    .unwrap_or_default();
    Some(ProxyInfo { server })
}

/// Secure DNS (DoH) настроен хотя бы на одном интерфейсе.
/// Windows хранит флаги в двух местах: DohFlags непосредственно в ключе
/// интерфейса либо в InterfaceSpecificParameters\<guid>\DohInterfaceSettings\
/// \<IP>\DohFlags (именно туда пишут Параметры Windows 11). Проверяем оба.
pub fn secure_dns_configured() -> bool {
    let key = match reg_open(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Services\Dnscache\InterfaceSpecificParameters",
    ) {
        Some(k) => k,
        None => return false,
    };
    let mut found = false;
    let mut idx = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        // SAFETY: key — валидный открытый ключ; name — буфер 256 WCHAR,
        // name_len — его размер на входе.
        let rc = unsafe {
            RegEnumKeyExW(
                key,
                idx,
                name.as_mut_ptr(),
                &mut name_len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            break;
        }
        let sub = String::from_utf16_lossy(&name[..name_len as usize]);
        if let Some(subkey) = reg_open_key_handle(key, &sub) {
            found = reg_read_dword_handle(subkey, "DohFlags").unwrap_or(0) > 0
                || doh_settings_have_flags(subkey);
            // SAFETY: subkey — открытый ключ, закрываем один раз.
            unsafe { RegCloseKey(subkey) };
        }
        if found {
            break;
        }
        idx += 1;
    }
    // SAFETY: key — открытый ключ, закрываем один раз.
    unsafe { RegCloseKey(key) };
    found
}

/// Проверить DohFlags в подключах DohInterfaceSettings\<IP> интерфейса.
fn doh_settings_have_flags(iface_key: HKEY) -> bool {
    let Some(settings) = reg_open_key_handle(iface_key, "DohInterfaceSettings") else {
        return false;
    };
    let mut found = false;
    let mut idx = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        // SAFETY: settings — валидный открытый ключ; name/name_len — буфер.
        let rc = unsafe {
            RegEnumKeyExW(
                settings,
                idx,
                name.as_mut_ptr(),
                &mut name_len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            break;
        }
        let sub = String::from_utf16_lossy(&name[..name_len as usize]);
        if let Some(server_key) = reg_open_key_handle(settings, &sub) {
            if reg_read_dword_handle(server_key, "DohFlags").unwrap_or(0) > 0 {
                found = true;
            }
            // SAFETY: server_key — открытый ключ, закрываем один раз.
            unsafe { RegCloseKey(server_key) };
        }
        if found {
            break;
        }
        idx += 1;
    }
    // SAFETY: settings — открытый ключ, закрываем один раз.
    unsafe { RegCloseKey(settings) };
    found
}

/// BFE (Base Filtering Engine) запущена — обязательна для WinDivert
/// (иначе EPT_S_NOT_REGISTERED, см. WINDIVERT_INTEGRATION п.6).
pub fn bfe_running() -> Result<bool, WindowsError> {
    match crate::windows::components::service_status("BFE")? {
        crate::windows::components::ServiceStatus::Running => Ok(true),
        _ => Ok(false),
    }
}

/// TCP timestamps включены (нужны для fooling=ts).
/// Чтение: netsh interface tcp show global. Вывод локализован —
/// ищем и английские, и русские маркеры.
pub fn tcp_timestamps_enabled() -> Result<bool, WindowsError> {
    let out = std::process::Command::new("netsh")
        .args(["interface", "tcp", "show", "global"])
        .output()
        .map_err(|_| WindowsError::NetshFailed)?;
    if !out.status.success() {
        return Err(WindowsError::NetshFailed);
    }
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for line in text.lines() {
        let is_ts = line.contains("timestamps") || line.contains("отметки времени");
        if is_ts {
            let enabled =
                line.contains("enabled") || line.contains("включен") || line.contains("включено");
            return Ok(enabled);
        }
    }
    Ok(false)
}

/// Включить TCP timestamps (только по подтверждению пользователя).
pub fn enable_tcp_timestamps() -> Result<(), WindowsError> {
    crate::windows::permissions::require_elevated()?;
    let status = std::process::Command::new("netsh")
        .args(["interface", "tcp", "set", "global", "timestamps=enabled"])
        .status()
        .map_err(|_| WindowsError::NetshFailed)?;
    if status.success() {
        Ok(())
    } else {
        Err(WindowsError::NetshFailed)
    }
}

/// Путь содержит кириллицу (известная проблема оригинала).
pub fn path_has_cyrillic(p: &Path) -> bool {
    p.to_string_lossy()
        .chars()
        .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

/// Путь внутри OneDrive.
pub fn is_onedrive_path(p: &Path) -> bool {
    match std::env::var("OneDrive") {
        Ok(od) if !od.is_empty() => p.starts_with(&od),
        _ => false,
    }
}

// --- Реестр: внутренние помощники ---

fn reg_open(root: HKEY, subkey: &str) -> Option<HKEY> {
    let sub = wide(subkey);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: sub — валидная NUL-терминированная UTF-16 строка; key —
    // валидный указатель на HKEY.
    let rc = unsafe { RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut key) };
    if rc == 0 {
        Some(key)
    } else {
        None
    }
}

fn reg_open_key_handle(parent: HKEY, subkey: &str) -> Option<HKEY> {
    let sub = wide(subkey);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: см. reg_open.
    let rc = unsafe { RegOpenKeyExW(parent, sub.as_ptr(), 0, KEY_READ, &mut key) };
    if rc == 0 {
        Some(key)
    } else {
        None
    }
}

fn reg_read_dword(root: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let key = reg_open(root, subkey)?;
    let out = reg_read_dword_handle(key, value);
    // SAFETY: key — открытый ключ, закрываем один раз.
    unsafe { RegCloseKey(key) };
    out
}

fn reg_read_dword_handle(key: HKEY, value: &str) -> Option<u32> {
    let name = wide(value);
    let mut ty = 0u32;
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: key — валидный ключ; name — NUL-терминированная строка;
    // data — буфер размера u32, size — его размер.
    let rc = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            &mut data as *mut _ as *mut u8,
            &mut size,
        )
    };
    if rc == 0 && ty == REG_DWORD {
        Some(data)
    } else {
        None
    }
}

fn reg_read_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let key = reg_open(root, subkey)?;
    let name = wide(value);
    let mut ty = 0u32;
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: key — валидный ключ; buf — буфер 512 WCHAR, size — его
    // размер в байтах.
    let rc = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        )
    };
    // SAFETY: key — открытый ключ, закрываем один раз.
    unsafe { RegCloseKey(key) };
    if rc == 0 && ty == REG_SZ && size >= 2 {
        let len = (size as usize) / 2;
        let trimmed = buf[..len].iter().position(|&c| c == 0).unwrap_or(len);
        Some(String::from_utf16_lossy(&buf[..trimmed]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_detection() {
        assert!(path_has_cyrillic(Path::new(r"C:\пользователи\mxds")));
        assert!(path_has_cyrillic(Path::new(r"C:\Users\Ё")));
        assert!(!path_has_cyrillic(Path::new(r"C:\Tools\mxds")));
    }

    #[test]
    fn os_version_sane() {
        let v = os_version().unwrap();
        assert_eq!(v.major, 10);
        assert!(v.build >= 17763, "build {}", v.build);
        assert!(is_supported_os().unwrap());
    }

    #[test]
    fn tcp_timestamps_query_does_not_error() {
        // На хосте netsh доступен; проверяем сам факт ответа.
        let _ = tcp_timestamps_enabled().unwrap();
    }
}
