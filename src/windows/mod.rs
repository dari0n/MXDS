//! Windows Integration — изолированный слой ОС.
//! См. docs/WINDOWS_INTEGRATION.md.
//!
//! unsafe сосредоточен здесь (и в windivert/ffi.rs) с SAFETY-комментариями
//! (SECURITY_REVIEW п.7). Ни один системный вызов не паникует.

pub mod components;
pub mod errors;
pub mod integration;
pub mod ipc;
pub mod permissions;

pub use components::{
    conflicting_services, service_status, stop_and_delete, windivert_cleanup, ServiceManager,
    ServiceStatus, SingleInstance,
};
pub use errors::WindowsError;
pub use integration::{
    bfe_running, is_onedrive_path, is_supported_os, os_version, path_has_cyrillic,
    secure_dns_configured, system_proxy, tcp_timestamps_enabled, OsVersion, ProxyInfo,
};
pub use permissions::{is_elevated, require_elevated};

/// UTF-16 строка с NUL-терминатором для WinAPI.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Текущий код ошибки Windows.
pub(crate) fn last_error() -> u32 {
    // SAFETY: GetLastError безопасна, потокобезопасна, без параметров.
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wide_has_nul_terminator() {
        let w = super::wide("mxds");
        assert_eq!(w.len(), 5);
        assert_eq!(w[4], 0);
    }
}
