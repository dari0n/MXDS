//! Ошибки WinDivert с маппингом кодов GetLastError.
//! Таблица кодов: docs/WINDIVERT_INTEGRATION.md, раздел 6.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DivertError {
    #[error("файлы драйвера WinDivert не найдены (код 2)")]
    DriverNotFound,
    #[error("нужны права администратора (код 5)")]
    AccessDenied,
    #[error("некорректный фильтр или параметр WinDivert (код 87)")]
    InvalidParameter,
    #[error("недействительная подпись драйвера WinDivert (код 577)")]
    DriverSignatureInvalid,
    #[error("загружена несовместимая версия драйвера, требуется перезагрузка ОС (код 654)")]
    DriverFailedPriorUnload,
    #[error("драйвер заблокирован ПО безопасности или средой виртуализации (код 1275)")]
    DriverBlocked,
    #[error("служба Base Filtering Engine не запущена (код 1753)")]
    BfeNotRunning,
    #[error("версия драйвера {got_major}.{got_minor}, ожидалась {expected_major}.x")]
    VersionMismatch {
        expected_major: u64,
        got_major: u64,
        got_minor: u64,
    },
    #[error("ошибка ввода-вывода WinDivert (код {0})")]
    Io(u32),
    #[error("неизвестная ошибка WinDivert (код {0})")]
    Unknown(u32),
}

impl DivertError {
    /// Маппинг кода GetLastError после вызова WinDivert API.
    /// Вызывается сразу после неудачного FFI-вызова (ERROR_HANDLING п.4).
    pub fn from_os_code(code: u32) -> Self {
        match code {
            2 => DivertError::DriverNotFound,
            5 => DivertError::AccessDenied,
            87 => DivertError::InvalidParameter,
            577 => DivertError::DriverSignatureInvalid,
            654 => DivertError::DriverFailedPriorUnload,
            1060 => DivertError::DriverNotFound, // NO_INSTALL — в MXDS не используется
            1275 => DivertError::DriverBlocked,
            1753 => DivertError::BfeNotRunning,
            other => DivertError::Unknown(other),
        }
    }

    /// Рекомендация пользователю (ERROR_HANDLING п.3).
    pub fn recommendation(&self) -> Option<&'static str> {
        match self {
            DivertError::DriverNotFound => {
                Some("Проверьте, что WinDivert.dll и WinDivert64.sys лежат рядом с mxds.exe")
            }
            DivertError::AccessDenied => Some("Перезапустите MXDS от имени администратора"),
            DivertError::InvalidParameter => {
                Some("Внутренняя ошибка MXDS: сообщите разработчику (см. логи)")
            }
            DivertError::DriverSignatureInvalid => {
                Some("Замените WinDivert64.sys на файл из официального дистрибутива MXDS")
            }
            DivertError::DriverFailedPriorUnload => Some("Перезагрузите компьютер"),
            DivertError::DriverBlocked => {
                Some("Антивирус или политика безопасности блокирует драйвер: добавьте папку MXDS в исключения")
            }
            DivertError::BfeNotRunning => {
                Some("Запустите службу Base Filtering Engine (services.msc → BFE)")
            }
            DivertError::VersionMismatch { .. } => {
                Some("Обновите файлы WinDivert из дистрибутива MXDS")
            }
            DivertError::Io(_) | DivertError::Unknown(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_codes() {
        assert!(matches!(
            DivertError::from_os_code(2),
            DivertError::DriverNotFound
        ));
        assert!(matches!(
            DivertError::from_os_code(5),
            DivertError::AccessDenied
        ));
        assert!(matches!(
            DivertError::from_os_code(577),
            DivertError::DriverSignatureInvalid
        ));
        assert!(matches!(
            DivertError::from_os_code(1275),
            DivertError::DriverBlocked
        ));
        assert!(matches!(
            DivertError::from_os_code(1753),
            DivertError::BfeNotRunning
        ));
    }

    #[test]
    fn unknown_code_preserved() {
        assert!(matches!(
            DivertError::from_os_code(9999),
            DivertError::Unknown(9999)
        ));
    }

    #[test]
    fn known_errors_have_recommendations() {
        for code in [2u32, 5, 87, 577, 654, 1275, 1753] {
            assert!(DivertError::from_os_code(code).recommendation().is_some());
        }
    }
}
