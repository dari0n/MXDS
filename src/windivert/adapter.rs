//! Безопасная обёртка над WinDivert (docs/MODULE_CONTRACTS.md п.4).
//!
//! DivertHandle владеет хендлом драйвера: Drop → WinDivertClose.
//! Рабочий цикл пакетов и маппинг стратегий живут в engine/.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::errors::DivertError;
use super::ffi::{self, WinDivertDll, WindivertAddress};
use crate::windows::last_error;

/// Максимальный размер пакета на NETWORK-слое (MTU + служебный запас).
pub const MAX_PACKET_SIZE: usize = 0xFFFF;

/// Параметр очереди WinDivert (типизированный вместо raw u32).
#[derive(Debug, Clone, Copy)]
pub enum DivertParam {
    QueueLength(u64),
    QueueTimeMs(u64),
    /// QUEUE_SIZE в байтах: поставляемая сборка WinDivert
    /// (zapret-win-bundle) принимает байты, а не КБ (живой прогон
    /// Цикла 7: 65536..=8388608 ok, КБ-значения — InvalidParameter).
    QueueSizeBytes(u64),
}

impl DivertParam {
    fn raw(&self) -> (u32, u64) {
        match *self {
            DivertParam::QueueLength(v) => (ffi::PARAM_QUEUE_LENGTH, v),
            DivertParam::QueueTimeMs(v) => (ffi::PARAM_QUEUE_TIME, v),
            DivertParam::QueueSizeBytes(v) => (ffi::PARAM_QUEUE_SIZE, v),
        }
    }
}

/// Принятый пакет: данные + адрес.
pub struct Packet {
    pub data: Vec<u8>,
    pub address: WindivertAddress,
}

/// Хендл WinDivert. Потокобезопасность: WinDivert допускает concurrent
/// Recv/Send на одном хендле, но MXDS использует один рабочий поток
/// (engine), поэтому Sync не объявляем без нужды.
pub struct DivertHandle {
    handle: *mut c_void,
    dll: Arc<WinDivertDll>,
}

// SAFETY: хендл WinDivert — opaque-указатель; engine передаёт владение
// рабочему потоку один раз (Send нужен для spawn). Гонок нет по
// контракту использования: recv-цикл в одном потоке, send/shutdown —
// через &self из того же потока или координированно при остановке
// (WinDivertShutdown потокобезопасен по документации WinDivert).
unsafe impl Send for DivertHandle {}
unsafe impl Sync for DivertHandle {}

/// Путь к DLL по умолчанию: assets/windivert рядом с exe (fallback —
/// каталог exe; для тестов/разработки — assets/windivert проекта).
pub fn default_dll_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let bundled = exe_dir
        .join("assets")
        .join("windivert")
        .join("WinDivert.dll");
    for candidate in [
        bundled.clone(),
        exe_dir.join("WinDivert.dll"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("windivert")
            .join("WinDivert.dll"),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }
    bundled
}

impl DivertHandle {
    /// Открыть хендл с фильтром на NETWORK-слое (priority 0).
    /// Требует elevation; проверяет SHA256 бинарников перед загрузкой.
    pub fn open(filter: &str) -> Result<Self, DivertError> {
        Self::open_with_path(filter, &default_dll_path())
    }

    /// Открыть хендл с явным путём к DLL (тесты, нестандартные развёртки).
    pub fn open_with_path(filter: &str, dll_path: &Path) -> Result<Self, DivertError> {
        crate::windows::permissions::require_elevated().map_err(|_| DivertError::AccessDenied)?;
        let dll = Arc::new(WinDivertDll::load(dll_path)?);

        // Фильтр — ASCII-строка языка фильтров WinDivert, генерируется
        // engine из типизированной конфигурации (не из ввода напрямую).
        let c_filter =
            std::ffi::CString::new(filter.as_bytes()).map_err(|_| DivertError::InvalidParameter)?;
        // SAFETY: c_filter — валидная NUL-терминированная строка, живёт
        // до конца вызова; layer/priority/flags — константы дизайна.
        let handle = unsafe {
            (dll.open)(
                c_filter.as_ptr(),
                ffi::LAYER_NETWORK,
                ffi::PRIORITY_DEFAULT,
                0,
            )
        };
        if handle.is_null() {
            return Err(DivertError::from_os_code(last_error()));
        }
        Ok(Self { handle, dll })
    }

    /// Установить параметр очереди.
    pub fn set_param(&self, p: DivertParam) -> Result<(), DivertError> {
        let (param, value) = p.raw();
        // SAFETY: handle валиден; param/value — из типизированного enum.
        let ok = unsafe { (self.dll.set_param)(self.handle, param, value) };
        if ok == 0 {
            Err(DivertError::from_os_code(last_error()))
        } else {
            Ok(())
        }
    }

    /// Версия драйвера (major, minor) — проверка совместимости 2.x.
    pub fn driver_version(&self) -> Result<(u64, u64), DivertError> {
        let mut major = 0u64;
        let mut minor = 0u64;
        // SAFETY: handle валиден; указатели на стековые u64 валидны.
        unsafe {
            if (self.dll.get_param)(self.handle, ffi::PARAM_VERSION_MAJOR, &mut major) == 0 {
                return Err(DivertError::from_os_code(last_error()));
            }
            if (self.dll.get_param)(self.handle, ffi::PARAM_VERSION_MINOR, &mut minor) == 0 {
                return Err(DivertError::from_os_code(last_error()));
            }
        }
        Ok((major, minor))
    }

    /// Принять следующий пакет (блокирующий вызов). Буфер — полный MTU.
    pub fn recv(&self) -> Result<Packet, DivertError> {
        let mut buf = vec![0u8; MAX_PACKET_SIZE];
        let mut recv_len = 0u32;
        let mut addr = WindivertAddress {
            timestamp: 0,
            flags: 0,
            reserved2: 0,
            data: [0; 40],
        };
        // SAFETY: buf — валидный буфер MAX_PACKET_SIZE; recv_len/addr —
        // валидные указатели; handle открыт на NETWORK-слое.
        let ok = unsafe {
            (self.dll.recv)(
                self.handle,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                &mut recv_len,
                &mut addr,
            )
        };
        if ok == 0 {
            return Err(DivertError::from_os_code(last_error()));
        }
        buf.truncate(recv_len as usize);
        Ok(Packet {
            data: buf,
            address: addr,
        })
    }

    /// Отправить (инъектировать) пакет с адресом.
    /// Возвращает число отправленных байт.
    pub fn send(&self, packet: &[u8], addr: &WindivertAddress) -> Result<usize, DivertError> {
        let mut sent = 0u32;
        // SAFETY: packet — валидный буфер packet.len(); addr — валидная
        // структура адреса (обычно от recv или сконструированная для
        // инъекции).
        let ok = unsafe {
            (self.dll.send)(
                self.handle,
                packet.as_ptr() as *const c_void,
                packet.len() as u32,
                &mut sent,
                addr,
            )
        };
        if ok == 0 {
            Err(DivertError::from_os_code(last_error()))
        } else {
            Ok(sent as usize)
        }
    }

    /// Пересчитать checksums после модификации пакета
    /// (WINDIVERT_INTEGRATION п.7: checksum offload сбрасывает флаги).
    pub fn calc_checksums(&self, packet: &mut [u8], addr: &mut WindivertAddress) {
        // SAFETY: packet — валидный буфер; addr — валидная структура.
        // flags=0: пересчёт всех checksums без NO_CHECKSUM-хинтов.
        unsafe {
            (self.dll.calc_checksums)(
                packet.as_mut_ptr() as *mut c_void,
                packet.len() as u32,
                addr,
                0,
            );
        }
    }

    /// Уменьшить TTL/HopLimit пакета на 1 (для impostor-инъекций).
    /// false — TTL уже исчерпан, пакет не подлежит инъекции.
    pub fn decrement_ttl(&self, packet: &mut [u8]) -> bool {
        // SAFETY: packet — валидный буфер размера packet.len().
        unsafe {
            (self.dll.decrement_ttl)(packet.as_mut_ptr() as *mut c_void, packet.len() as u32) != 0
        }
    }

    /// Управляемая остановка приёма (RECV): рабочий цикл выходит
    /// из блокирующего recv, дочитывает очередь и закрывает хендл
    /// (graceful stop, WINDIVERT_INTEGRATION п.5).
    pub fn shutdown(&self) -> Result<(), DivertError> {
        // SAFETY: handle валиден; SHUTDOWN_RECV — документированный режим.
        let ok = unsafe { (self.dll.shutdown)(self.handle, ffi::SHUTDOWN_RECV) };
        if ok == 0 {
            Err(DivertError::from_os_code(last_error()))
        } else {
            Ok(())
        }
    }
}

impl Drop for DivertHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle открыт WinDivertOpen, закрываем ровно один
            // раз; после этого поле обнуляем.
            unsafe {
                (self.dll.close)(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_ranges_documented() {
        // Диапазоны из документации WinDivert 2.2 (см. ffi::QUEUE_*_MIN/MAX)
        // валидируются Config Manager; здесь — smoke на raw-маппинг.
        assert_eq!(
            DivertParam::QueueLength(4096).raw().0,
            ffi::PARAM_QUEUE_LENGTH
        );
        assert_eq!(
            DivertParam::QueueTimeMs(2000).raw().0,
            ffi::PARAM_QUEUE_TIME
        );
        assert_eq!(
            DivertParam::QueueSizeBytes(4194304).raw().0,
            ffi::PARAM_QUEUE_SIZE
        );
    }
}
