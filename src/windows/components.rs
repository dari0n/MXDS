//! Управление системными компонентами: SCM (службы), WinDivert-очистка,
//! single-instance (мьютекс + lock-файл с PID).
//! См. docs/WINDOWS_INTEGRATION.md, docs/MODULE_CONTRACTS.md п.5.
//!
//! Named Pipe IPC (`\\.\pipe\MXDS.Control`, D-009) — в `windows::ipc`.

use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
    OpenServiceW, QueryServiceStatus, StartServiceW, SC_HANDLE, SERVICE_STATUS,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, ReleaseMutex, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{last_error, wide, WindowsError};

// Константы SCM (в windows-sys частично отсутствуют — определяем явно).
const SC_MANAGER_ALL_ACCESS: u32 = 0xF003F;
const SERVICE_ALL_ACCESS: u32 = 0xF01FF;
const SERVICE_QUERY_STATUS: u32 = 0x0004;
const SERVICE_STOP: u32 = 0x0020;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x0000_0010;
const SERVICE_AUTO_START: u32 = 2;
const SERVICE_ERROR_NORMAL: u32 = 1;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;

/// Состояние службы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    NotFound,
    /// Любое иное состояние (StartPending, StopPending, Paused, ...).
    Other(u32),
}

/// Службы, конфликтующие с MXDS (из SECURITY_REVIEW п.5).
const CONFLICTING: &[&str] = &[
    "GoodbyeDPI",
    "discordfix_zapret",
    "winws1",
    "winws2",
    "zapret",
];

// --- RAII-обёртки над хендлами SCM ---

struct ScmHandle(SC_HANDLE);

impl Drop for ScmHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: хендл открыт OpenSCManagerW, закрываем ровно один раз.
            unsafe { CloseServiceHandle(self.0) };
        }
    }
}

struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: хендл открыт OpenServiceW/CreateServiceW, закрываем
            // ровно один раз.
            unsafe { CloseServiceHandle(self.0) };
        }
    }
}

/// Открыть SCM. Требует elevation для большинства операций.
fn open_scm() -> Result<ScmHandle, WindowsError> {
    // SAFETY: все указатели null — подключение к локальному SCM.
    let h = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS) };
    if h.is_null() {
        let code = last_error();
        if code == 5 {
            // ERROR_ACCESS_DENIED
            Err(WindowsError::NotElevated)
        } else {
            Err(WindowsError::Unknown(code))
        }
    } else {
        Ok(ScmHandle(h))
    }
}

fn open_service(
    scm: &ScmHandle,
    name: &str,
    access: u32,
) -> Result<Option<ServiceHandle>, WindowsError> {
    let wname = wide(name);
    // SAFETY: scm — валидный хендл SCM; wname — NUL-терминированная строка.
    let h = unsafe { OpenServiceW(scm.0, wname.as_ptr(), access) };
    if h.is_null() {
        let code = last_error();
        if code == ERROR_SERVICE_DOES_NOT_EXIST {
            Ok(None)
        } else if code == 5 {
            Err(WindowsError::NotElevated)
        } else {
            Err(WindowsError::Service {
                op: "open",
                name: name.to_string(),
                code,
            })
        }
    } else {
        Ok(Some(ServiceHandle(h)))
    }
}

/// Запросить состояние службы (не требует elevation).
pub fn service_status(name: &str) -> Result<ServiceStatus, WindowsError> {
    // Для QUERY_STATUS достаточно SC_MANAGER_CONNECT, но OpenSCManagerW
    // с ALL_ACCESS упадёт без прав — открываем с минимальным доступом.
    const SC_MANAGER_CONNECT: u32 = 0x0001;
    // SAFETY: см. open_scm.
    let h = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if h.is_null() {
        return Err(WindowsError::Unknown(last_error()));
    }
    let scm = ScmHandle(h);
    let Some(svc) = open_service(&scm, name, SERVICE_QUERY_STATUS)? else {
        return Ok(ServiceStatus::NotFound);
    };
    // SAFETY: SERVICE_STATUS — POD-структура, нулевая инициализация валидна.
    let mut status = unsafe { std::mem::zeroed::<SERVICE_STATUS>() };
    // SAFETY: svc — валидный хендл службы с SERVICE_QUERY_STATUS;
    // status — валидный буфер SERVICE_STATUS.
    let ok = unsafe { QueryServiceStatus(svc.0, &mut status) };
    if ok == 0 {
        return Err(WindowsError::Service {
            op: "query",
            name: name.to_string(),
            code: last_error(),
        });
    }
    Ok(match status.dwCurrentState {
        4 => ServiceStatus::Running, // SERVICE_RUNNING
        1 => ServiceStatus::Stopped, // SERVICE_STOPPED
        s => ServiceStatus::Other(s),
    })
}

/// Список присутствующих конфликтующих служб (GoodbyeDPI, zapret, ...).
/// Ошибки отдельных проверок проглатываются — это диагностика.
pub fn conflicting_services() -> Vec<String> {
    CONFLICTING
        .iter()
        .filter(|name| !matches!(service_status(name), Ok(ServiceStatus::NotFound) | Err(_)))
        .map(|s| s.to_string())
        .collect()
}

/// Остановить и удалить службу, если существует. Отсутствие — не ошибка.
pub fn stop_and_delete(name: &str) -> Result<(), WindowsError> {
    let scm = open_scm()?;
    let Some(svc) = open_service(&scm, name, SERVICE_ALL_ACCESS)? else {
        return Ok(());
    };
    // SAFETY: SERVICE_STATUS — POD-структура, нулевая инициализация валидна.
    let mut status = unsafe { std::mem::zeroed::<SERVICE_STATUS>() };
    if unsafe { QueryServiceStatus(svc.0, &mut status) } != 0 && status.dwCurrentState != 1 {
        // Служба не остановлена — шлём STOP. Уже остановленная служба
        // вернёт ошибку, это нормально, игнорируем код.
        // SAFETY: svc — валидный хендл; status — валидный буфер.
        unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) };
    }
    // SAFETY: svc — валидный хендл службы, открытый с DELETE-доступом
    // (входит в SERVICE_ALL_ACCESS).
    if unsafe { DeleteService(svc.0) } == 0 {
        let code = last_error();
        if code != ERROR_SERVICE_DOES_NOT_EXIST {
            return Err(WindowsError::Service {
                op: "delete",
                name: name.to_string(),
                code,
            });
        }
    }
    Ok(())
}

/// Восстановление после сбоя: снять «осиротевший» драйвер WinDivert,
/// оставшийся от предыдущего запуска (WINDOWS_INTEGRATION п.7).
pub fn windivert_cleanup() -> Result<(), WindowsError> {
    for name in ["WinDivert", "WinDivert14"] {
        stop_and_delete(name)?;
    }
    Ok(())
}

/// Имена всех Win32-служб системы (для диагностических substring-проверок:
/// Killer, SmartByte, Check Point, VPN и т.п.). Elevation не требуется.
pub fn enumerate_services() -> Result<Vec<String>, WindowsError> {
    use windows_sys::Win32::System::Services::{
        EnumServicesStatusExW, ENUM_SERVICE_STATUS_PROCESSW,
    };
    const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x0004;
    const SC_ENUM_PROCESS_INFO: i32 = 0; // SC_ENUM_TYPE
    const SERVICE_WIN32: u32 = 0x30; // SERVICE_WIN32_OWN_PROCESS | SHARE_PROCESS
    const SERVICE_STATE_ALL: u32 = 3;
    const ERROR_MORE_DATA: u32 = 234;

    // SAFETY: указатели null — локальный SCM; хендл сразу в RAII.
    let h = unsafe {
        OpenSCManagerW(
            std::ptr::null(),
            std::ptr::null(),
            SC_MANAGER_ENUMERATE_SERVICE,
        )
    };
    if h.is_null() {
        return Err(WindowsError::Unknown(last_error()));
    }
    let scm = ScmHandle(h);

    let mut bytes_needed = 0u32;
    let mut returned = 0u32;
    let mut resume = 0u32;
    // SAFETY: нулевой буфер — запрос требуемого размера; ожидаем
    // ERROR_MORE_DATA. Все out-параметры указывают на живые переменные.
    unsafe {
        EnumServicesStatusExW(
            scm.0,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_STATE_ALL,
            std::ptr::null_mut(),
            0,
            &mut bytes_needed,
            &mut returned,
            &mut resume,
            std::ptr::null(),
        );
    }
    let code = last_error();
    if bytes_needed == 0 {
        // Либо служб нет (практически невозможно), либо настоящая ошибка.
        return if code == 0 || code == ERROR_MORE_DATA {
            Ok(Vec::new())
        } else {
            Err(WindowsError::Unknown(code))
        };
    }
    let mut buf = vec![0u8; bytes_needed as usize];
    let mut returned = 0u32;
    resume = 0;
    // SAFETY: buf имеет ровно bytes_needed байт; записи
    // ENUM_SERVICE_STATUS_PROCESSW и строки размещаются внутри buf,
    // читаем их до уничтожения buf.
    let ok = unsafe {
        EnumServicesStatusExW(
            scm.0,
            SC_ENUM_PROCESS_INFO,
            SERVICE_WIN32,
            SERVICE_STATE_ALL,
            buf.as_mut_ptr(),
            bytes_needed,
            &mut bytes_needed,
            &mut returned,
            &mut resume,
            std::ptr::null(),
        )
    };
    if ok == 0 {
        return Err(WindowsError::Unknown(last_error()));
    }
    // SAFETY: EnumServicesStatusExW заполнил ровно `returned` записей
    // в начале buf; выравнивание гарантировано Vec<u8> с запасом
    // (буфер выделен под структуры API). lpServiceName указывает внутрь
    // buf, который жив до конца функции.
    let entries = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
            returned as usize,
        )
    };
    let mut names = Vec::with_capacity(entries.len());
    for e in entries {
        // SAFETY: lpServiceName — NUL-терминированная строка внутри buf.
        names.push(unsafe { pwstr_to_string(e.lpServiceName) });
    }
    Ok(names)
}

/// Прочитать NUL-терминированную wide-строку.
/// SAFETY: вызывающий гарантирует валидность указателя и NUL-терминацию.
unsafe fn pwstr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: строка NUL-терминирована по контракту вызывающего.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

/// Менеджер службы `mxds` (автозапуск по D-008).
pub struct ServiceManager;

impl ServiceManager {
    pub const SERVICE_NAME: &'static str = "mxds";

    /// Состояние службы mxds.
    pub fn status() -> Result<ServiceStatus, WindowsError> {
        service_status(Self::SERVICE_NAME)
    }

    /// Установить службу (автозапуск, свой процесс). Требует elevation.
    pub fn install(exe_path: &std::path::Path, display_name: &str) -> Result<(), WindowsError> {
        super::permissions::require_elevated()?;
        let scm = open_scm()?;
        let wname = wide(Self::SERVICE_NAME);
        let wdisplay = wide(display_name);
        let wpath = wide(&exe_path.to_string_lossy());
        // SAFETY: все строки NUL-терминированные; scm — валидный хендл.
        // Возвращаемый хендл сразу оборачиваем в RAII.
        let h = unsafe {
            CreateServiceW(
                scm.0,
                wname.as_ptr(),
                wdisplay.as_ptr(),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START,
                SERVICE_ERROR_NORMAL,
                wpath.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if h.is_null() {
            return Err(WindowsError::Service {
                op: "create",
                name: Self::SERVICE_NAME.to_string(),
                code: last_error(),
            });
        }
        let _svc = ServiceHandle(h); // закроется в Drop
        Ok(())
    }

    /// Запустить службу. Уже запущенная — не ошибка.
    pub fn start() -> Result<(), WindowsError> {
        super::permissions::require_elevated()?;
        let scm = open_scm()?;
        let Some(svc) = open_service(&scm, Self::SERVICE_NAME, SERVICE_ALL_ACCESS)? else {
            return Err(WindowsError::Service {
                op: "start",
                name: Self::SERVICE_NAME.to_string(),
                code: ERROR_SERVICE_DOES_NOT_EXIST,
            });
        };
        // SAFETY: svc — валидный хендл с SERVICE_START (в ALL_ACCESS).
        let ok = unsafe { StartServiceW(svc.0, 0, std::ptr::null()) };
        if ok == 0 {
            let code = last_error();
            // 1056 = ERROR_SERVICE_ALREADY_RUNNING
            if code != 1056 {
                return Err(WindowsError::Service {
                    op: "start",
                    name: Self::SERVICE_NAME.to_string(),
                    code,
                });
            }
        }
        Ok(())
    }

    /// Остановить службу. Уже остановленная/отсутствующая — не ошибка.
    pub fn stop() -> Result<(), WindowsError> {
        super::permissions::require_elevated()?;
        let scm = open_scm()?;
        let Some(svc) = open_service(&scm, Self::SERVICE_NAME, SERVICE_STOP)? else {
            return Ok(());
        };
        // SAFETY: SERVICE_STATUS — POD-структура, нулевая инициализация валидна.
        let mut status = unsafe { std::mem::zeroed::<SERVICE_STATUS>() };
        // SAFETY: svc — валидный хендл с SERVICE_STOP.
        if unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) } == 0 {
            let code = last_error();
            // 1062 = ERROR_SERVICE_NOT_ACTIVE
            if code != 1062 {
                return Err(WindowsError::Service {
                    op: "stop",
                    name: Self::SERVICE_NAME.to_string(),
                    code,
                });
            }
        }
        Ok(())
    }

    /// Остановить и удалить службу mxds.
    pub fn remove() -> Result<(), WindowsError> {
        super::permissions::require_elevated()?;
        stop_and_delete(Self::SERVICE_NAME)
    }
}

/// Единственный экземпляр приложения: мьютекс `Global\MXDS.SingleInstance`
/// + lock-файл с PID (WINDOWS_INTEGRATION п.7). Drop освобождает оба.
pub struct SingleInstance {
    mutex: windows_sys::Win32::Foundation::HANDLE,
    lock_path: PathBuf,
}

impl SingleInstance {
    /// Захватить экземпляр. Err(InstanceAlreadyRunning), если мьютекс занят
    /// И процесс из lock-файла жив.
    pub fn acquire() -> Result<Self, WindowsError> {
        let wname = wide(r"Global\MXDS.SingleInstance");
        // SAFETY: wname — NUL-терминированная строка; права на мьютекс
        // не запрашиваем (создание).
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, wname.as_ptr()) };
        if mutex.is_null() {
            return Err(WindowsError::Unknown(last_error()));
        }
        let already = last_error() == ERROR_ALREADY_EXISTS;

        let lock_path = Self::lock_path();
        if already {
            // Мьютекс занят. Проверяем живость процесса из lock-файла:
            // если процесс умер, а мьютекс остался (краш до ReleaseMutex
            // всё равно освобождает мьютекс ядром, но страхуемся) —
            // считаем экземпляр свободным.
            if Self::lock_owner_alive(&lock_path) {
                // SAFETY: mutex — открытый хендл, закрываем один раз.
                unsafe { CloseHandle(mutex) };
                return Err(WindowsError::InstanceAlreadyRunning);
            }
            tracing::warn!("мьютекс single-instance занят, но владелец мёртв — продолжаем");
        }

        // Пишем свой PID в lock-файл (best effort).
        let pid = std::process::id();
        if let Err(e) = std::fs::write(&lock_path, pid.to_string()) {
            tracing::warn!(?e, ?lock_path, "не удалось записать lock-файл");
        }

        Ok(Self { mutex, lock_path })
    }

    /// Путь lock-файла: рядом с exe (fallback — temp).
    fn lock_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("mxds.lock")))
            .unwrap_or_else(|| std::env::temp_dir().join("mxds.lock"))
    }

    /// Жив ли процесс-владелец из lock-файла.
    fn lock_owner_alive(path: &PathBuf) -> bool {
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(pid) = content.trim().parse::<u32>() else {
            return false;
        };
        // SAFETY: pid из lock-файла; запрашиваем минимальный доступ.
        // Живой процесс (в т.ч. наш собственный при повторном захвате)
        // означает: экземпляр занят.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if h.is_null() {
            return false;
        }
        // SAFETY: h — открытый хендл процесса, закрываем один раз.
        unsafe { CloseHandle(h) };
        true
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: mutex — наш хендл; ReleaseMutex для невладеющего
        // потока вернёт ошибку, что безопасно; CloseHandle — один раз.
        unsafe {
            ReleaseMutex(self.mutex);
            CloseHandle(self.mutex);
        }
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

// SAFETY: HANDLE — ядерный объект ОС, операции над ним
// потокобезопасны; PathBuf сам по себе Send+Sync. CoreManager
// (Цикл 14) хранит экземпляр и используется из async-рантайма.
unsafe impl Send for SingleInstance {}
unsafe impl Sync for SingleInstance {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Сериализация тестов, захватывающих глобальный мьютекс
    /// single-instance (он один на весь тестовый процесс).
    /// Используется также тестами application::manager.
    pub(crate) static SI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn single_instance_acquire_and_second_fails() {
        let _serialization = SI_TEST_LOCK.lock().unwrap();
        let first = SingleInstance::acquire().expect("первый захват");
        let second = SingleInstance::acquire();
        assert!(matches!(second, Err(WindowsError::InstanceAlreadyRunning)));
        drop(first);
        // После Drop мьютекс освобождён — можно захватить снова.
        let third = SingleInstance::acquire().expect("захват после освобождения");
        drop(third);
    }

    #[test]
    fn service_status_absent_service_is_not_found() {
        let st = service_status("mxds_definitely_absent_service_xyz").unwrap();
        assert_eq!(st, ServiceStatus::NotFound);
    }

    #[test]
    fn conflicting_services_does_not_panic() {
        let _ = conflicting_services();
    }
}
