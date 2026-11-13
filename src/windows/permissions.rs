//! Проверка прав администратора (WINDOWS_INTEGRATION п.2).
//!
//! Реализация: OpenProcessToken + GetTokenInformation(TokenElevation).

use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::windows::last_error;

/// Фактическая проверка elevation текущего процесса.
/// При ошибке WinAPI возвращает false — безопасная деградация
/// (операция всё равно упадёт с AccessDenied и понятной рекомендацией).
pub fn is_elevated() -> bool {
    let mut token = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess возвращает псевдохендл текущего процесса,
    // он всегда валиден. token — валидный указатель на HANDLE.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        tracing::warn!(code = last_error(), "OpenProcessToken failed");
        return false;
    }

    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut ret_len = 0u32;
    // SAFETY: token — валидный хендл токена; elevation — валидный буфер
    // размера TOKEN_ELEVATION; ret_len — валидный указатель.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
    };
    // SAFETY: token — открытый хендл, закрываем ровно один раз.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };

    ok != 0 && elevation.TokenIsElevated != 0
}

/// Проверка elevation с ошибкой (для операций, требующих прав).
pub fn require_elevated() -> Result<(), crate::windows::WindowsError> {
    if is_elevated() {
        Ok(())
    } else {
        Err(crate::windows::WindowsError::NotElevated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_elevated_does_not_panic() {
        // Значение зависит от контекста запуска тестов; важна отсутствие паники.
        let _ = is_elevated();
    }
}
