//! Named Pipe IPC-транспорт (D-009): `\\.\pipe\MXDS.Control`.
//! Сервер создаётся с DACL «только текущий пользователь»; обмен —
//! JSON-lines (одна строка = одна команда/ответ). Транспорт синхронный
//! (Win32); асинхронность обеспечивает вызывающий (spawn_blocking).
//!
//! Сетевых сокетов нет; пайп доступен только локальному пользователю,
//! создавшему сервер.

use std::io;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PIPE_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{last_error, wide};

/// Имя пайпа координации (D-009).
pub const PIPE_NAME: &str = r"\\.\pipe\MXDS.Control";

/// Максимальный размер одного JSON-сообщения (1 МиБ).
pub const MAX_MESSAGE: usize = 1024 * 1024;

// Константы (в windows-sys частично отсутствуют как типизированные).
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
const PIPE_WAIT: u32 = 0x0000_0000;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SDDL_REVISION_1: u32 = 1;

fn io_err(op: &'static str) -> io::Error {
    let code = last_error();
    io::Error::other(format!("{op}: код Windows {code}"))
}

/// SID текущего пользователя в SDDL-представлении (S-1-5-...).
fn current_user_sid_string() -> Result<String, super::WindowsError> {
    // SAFETY: GetCurrentProcess — псевдохендл текущего процесса;
    // token — валидный буфер хендла.
    let mut token: HANDLE = std::ptr::null_mut();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(super::WindowsError::Unknown(last_error()));
    }
    struct TokenGuard(HANDLE);
    impl Drop for TokenGuard {
        fn drop(&mut self) {
            // SAFETY: хендл открыт OpenProcessToken, закрываем один раз.
            unsafe { CloseHandle(self.0) };
        }
    }
    let token = TokenGuard(token);

    // Размер буфера TOKEN_USER.
    let mut needed = 0u32;
    // SAFETY: нулевой буфер — запрос размера; ожидаем ошибку с needed > 0.
    unsafe {
        GetTokenInformation(
            token.0,
            windows_sys::Win32::Security::TokenUser,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return Err(super::WindowsError::Unknown(last_error()));
    }
    let mut buf = vec![0u8; needed as usize];
    // SAFETY: buf имеет needed байт; token валиден.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            windows_sys::Win32::Security::TokenUser,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(super::WindowsError::Unknown(last_error()));
    }
    // SAFETY: буфер заполнен TOKEN_USER; Sid валиден до конца функции.
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut sid_str: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: Sid — валидный SID из токена; sid_str получит строку,
    // которую освобождаем LocalFree.
    let ok = unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_str) };
    if ok == 0 {
        return Err(super::WindowsError::Unknown(last_error()));
    }
    // SAFETY: sid_str — NUL-терминированная wide-строка от ConvertSid*.
    let s = unsafe {
        let mut len = 0usize;
        while *sid_str.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(sid_str, len))
    };
    // SAFETY: sid_str выделена ConvertSidToStringSidW — освобождаем LocalFree.
    unsafe { LocalFree(sid_str as *mut core::ffi::c_void) };
    Ok(s)
}

/// SECURITY_ATTRIBUTES с DACL «полный доступ только текущему
/// пользователю» (D:P(A;;GA;;;<SID>)). Security descriptor освобождается
/// в Drop.
pub struct CurrentUserSecurity {
    sd: *mut core::ffi::c_void,
    attrs: SECURITY_ATTRIBUTES,
}

impl CurrentUserSecurity {
    pub fn new() -> Result<Self, super::WindowsError> {
        let sid = current_user_sid_string()?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let wsddl = wide(&sddl);
        let mut sd: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: wsddl — валидная SDDL-строка; sd получит дескриптор,
        // освобождаемый LocalFree в Drop.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wsddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(super::WindowsError::Unknown(last_error()));
        }
        Ok(Self {
            sd,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            },
        })
    }

    fn attrs_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attrs
    }
}

impl Drop for CurrentUserSecurity {
    fn drop(&mut self) {
        if !self.sd.is_null() {
            // SAFETY: sd выделен ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe { LocalFree(self.sd) };
        }
    }
}

/// Одно подключение пайпа (со стороны сервера или клиента).
pub struct PipeStream {
    handle: HANDLE,
}

// SAFETY: хендл пайпа не привязан к потоку-создателю; ReadFile/WriteFile
// потокобезопасны при последовательном использовании (гарантируется &mut).
unsafe impl Send for PipeStream {}

impl Drop for PipeStream {
    fn drop(&mut self) {
        if !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: хендл валиден, закрываем ровно один раз.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

impl PipeStream {
    /// Прочитать одну строку (до '\n', лимит MAX_MESSAGE).
    pub fn read_line(&mut self) -> io::Result<String> {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            let mut read = 0u32;
            // SAFETY: handle валиден; chunk — буфер 4096 байт.
            let ok = unsafe {
                ReadFile(
                    self.handle,
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io_err("pipe read"));
            }
            buf.extend_from_slice(&chunk[..read as usize]);
            if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                buf.truncate(pos);
                break;
            }
            if buf.len() > MAX_MESSAGE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "сообщение пайпа превышает 1 МиБ",
                ));
            }
            if read == 0 {
                break; // EOF
            }
        }
        String::from_utf8(buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("utf8: {e}")))
    }

    /// Записать строку + '\n' и сбросить буфер.
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let mut data = line.as_bytes().to_vec();
        data.push(b'\n');
        let mut written_total = 0usize;
        while written_total < data.len() {
            let mut written = 0u32;
            // SAFETY: handle валиден; data жив на время вызова.
            let ok = unsafe {
                WriteFile(
                    self.handle,
                    data[written_total..].as_ptr(),
                    (data.len() - written_total) as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io_err("pipe write"));
            }
            written_total += written as usize;
        }
        // SAFETY: handle валиден.
        if unsafe { FlushFileBuffers(self.handle) } == 0 {
            return Err(io_err("pipe flush"));
        }
        Ok(())
    }

    /// Завершить серверную сторону: клиент получит EOF, хендл
    /// переиспользования нет (следующий accept создаёт новый экземпляр).
    pub fn disconnect(self) {
        // SAFETY: handle валиден; ошибки не интересны (клиент мог отвалиться).
        unsafe { DisconnectNamedPipe(self.handle) };
    }
}

/// Серверная сторона: принять одно подключение (блокирующе).
/// Каждый вызов создаёт новый экземпляр пайпа с DACL текущего
/// пользователя и ждёт клиента.
pub fn accept(pipe_name: &str) -> io::Result<PipeStream> {
    let security = CurrentUserSecurity::new().map_err(|e| io::Error::other(e.to_string()))?;
    let wname = wide(pipe_name);
    // SAFETY: все указатели валидны; security живёт до конца вызова.
    let handle = unsafe {
        CreateNamedPipeW(
            wname.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            security.attrs_ptr(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io_err("CreateNamedPipeW"));
    }
    // SAFETY: handle валиден; lpOverlapped null — синхронное ожидание.
    let ok = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        // Клиент успел подключиться между Create и Connect — норма.
        if code != ERROR_PIPE_CONNECTED {
            // SAFETY: handle валиден.
            unsafe { CloseHandle(handle) };
            return Err(io::Error::other(format!(
                "ConnectNamedPipe: код Windows {code}"
            )));
        }
    }
    Ok(PipeStream { handle })
}

/// Клиентская сторона: подключиться к работающему экземпляру.
/// Err(NotFound) — сервера нет (экземпляр не запущен).
pub fn connect(pipe_name: &str) -> io::Result<PipeStream> {
    let wname = wide(pipe_name);
    // SAFETY: wname — NUL-терминированная строка; шаблоны/наследование
    // не используются.
    let handle = unsafe {
        CreateFileW(
            wname.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let code = unsafe { GetLastError() };
        let kind = if code == ERROR_FILE_NOT_FOUND {
            io::ErrorKind::NotFound
        } else {
            io::ErrorKind::Other
        };
        return Err(io::Error::new(
            kind,
            format!("pipe connect: код Windows {code}"),
        ));
    }
    Ok(PipeStream { handle })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_roundtrip() {
        // Уникальное имя, чтобы не пересекаться с живым экземпляром
        // и параллельными тестами.
        let name = format!(r"\\.\pipe\MXDS.Test.{}", std::process::id());
        let server_name = name.clone();
        let server = std::thread::spawn(move || {
            let mut stream = accept(&server_name).unwrap();
            let line = stream.read_line().unwrap();
            assert_eq!(line, r#"{"cmd":"status"}"#);
            stream.write_line(r#"{"ok":true}"#).unwrap();
            stream.disconnect();
        });
        // Серверу нужно время создать пайп; подключаемся с retry.
        let mut client = None;
        for _ in 0..50 {
            match connect(&name) {
                Ok(c) => {
                    client = Some(c);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        let mut client = client.expect("клиент подключился");
        client.write_line(r#"{"cmd":"status"}"#).unwrap();
        let reply = client.read_line().unwrap();
        assert_eq!(reply, r#"{"ok":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn connect_missing_pipe_is_not_found() {
        let r = connect(r"\\.\pipe\MXDS.Test.NoSuchPipe");
        assert!(matches!(r, Err(e) if e.kind() == io::ErrorKind::NotFound));
    }

    #[test]
    fn sddl_builds_for_current_user() {
        let sid = current_user_sid_string().unwrap();
        assert!(sid.starts_with("S-1-"));
        let _security = CurrentUserSecurity::new().unwrap();
    }
}
