//! IPC-клиент: короткие команды работающему экземпляру (D-009).
//! Отсутствие сервера (экземпляр не запущен) — Ok(None): вызывающий
//! переходит в автономный режим.

use std::io;

use crate::windows::ipc;

use super::protocol::{encode_command, IpcCommand, IpcResponse};

/// Отправить команду работающему экземпляру.
/// Ok(None) — сервера нет; Ok(Some(...)) — ответ; Err — сбой транспорта.
pub fn try_send(cmd: &IpcCommand) -> io::Result<Option<IpcResponse>> {
    try_send_named(ipc::PIPE_NAME, cmd)
}

/// Вариант с явным именем пайпа (тесты).
pub fn try_send_named(pipe_name: &str, cmd: &IpcCommand) -> io::Result<Option<IpcResponse>> {
    let mut stream = match ipc::connect(pipe_name) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    stream.write_line(&encode_command(cmd))?;
    let line = stream.read_line()?;
    let resp = serde_json::from_str(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("IPC ответ: {e}")))?;
    Ok(Some(resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_server_is_none() {
        let r = try_send_named(r"\\.\pipe\MXDS.Test.NoServer", &IpcCommand::Status).unwrap();
        assert!(r.is_none());
    }
}
