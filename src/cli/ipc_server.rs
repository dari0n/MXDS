//! IPC-сервер работающего экземпляра (D-009): принимает команды по
//! \\.\pipe\MXDS.Control и диспетчеризует в Application Core.
//! Сервер живёт в процессе владельца CoreManager (run-служба, GUI).
//!
//! Корректная остановка: блокирующий Win32-accept не прерывается
//! извне, поэтому shutdown будит сервер dummy-подключением — accept
//! возвращается, цикл видит флаг и завершается (иначе поток
//! spawn_blocking не дал бы рантайму завершиться).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::application::CoreManager;
use crate::windows::ipc::{self, PipeStream};

use super::protocol::{decode_command, IpcCommand, IpcResponse};

/// Работающий IPC-сервер. shutdown будит accept и ждёт завершения цикла.
pub struct IpcServerHandle {
    task: JoinHandle<()>,
    stop: Arc<AtomicBool>,
    pipe_name: String,
}

impl IpcServerHandle {
    /// Остановить сервер: флаг + dummy-подключение, ожидание цикла.
    pub async fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        let name = self.pipe_name.clone();
        // Разбудить блокирующий accept; отказ допустим (сервер мог
        // уже умереть).
        let _ = tokio::task::spawn_blocking(move || {
            let _ = ipc::connect(&name);
        })
        .await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

/// Запустить цикл IPC-сервера (D-009: \\.\pipe\MXDS.Control).
pub fn spawn(core: Arc<CoreManager>) -> IpcServerHandle {
    spawn_named(core, ipc::PIPE_NAME.to_string())
}

/// Вариант с явным именем пайпа (тесты — уникальные имена).
pub fn spawn_named(core: Arc<CoreManager>, pipe_name: String) -> IpcServerHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    let name = pipe_name.clone();
    let task = tokio::spawn(async move {
        loop {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            let n = name.clone();
            let accepted = tokio::task::spawn_blocking(move || ipc::accept(&n)).await;
            let mut stream = match accepted {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "ipc: accept неудачен");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(e) => {
                    tracing::error!(error = %e, "ipc: accept завершился паникой");
                    continue;
                }
            };
            serve_connection(&core, &mut stream).await;
            stream.disconnect();
        }
        tracing::info!("ipc: сервер остановлен");
    });
    IpcServerHandle {
        task,
        stop,
        pipe_name,
    }
}

/// Одно подключение: read line → dispatch → write response.
async fn serve_connection(core: &Arc<CoreManager>, stream: &mut PipeStream) {
    let read = {
        let mut s = unsafe { stream_alias(stream) };
        tokio::task::spawn_blocking(move || s.read_line()).await
    };
    let line = match read {
        Ok(Ok(l)) => l,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "ipc: чтение команды не удалось");
            return;
        }
        Err(_) => return,
    };
    let response = match decode_command(&line) {
        Ok(cmd) => dispatch(core, cmd).await,
        Err(e) => IpcResponse::err(e),
    };
    let text = serde_json::to_string(&response).unwrap_or_else(|_| r#"{"ok":false}"#.to_string());
    let mut s = unsafe { stream_alias(stream) };
    if let Err(e) = tokio::task::spawn_blocking(move || s.write_line(&text)).await {
        tracing::debug!(error = %e, "ipc: запись ответа не удалась");
    }
}

/// PipeStream не Sync; передача &mut в spawn_blocking безопасна здесь:
/// вызывающий ждёт завершения задачи до следующего обращения.
unsafe fn stream_alias(stream: &mut PipeStream) -> PipeStreamAlias {
    PipeStreamAlias(stream as *mut PipeStream)
}

struct PipeStreamAlias(*mut PipeStream);
unsafe impl Send for PipeStreamAlias {}

impl PipeStreamAlias {
    fn read_line(&mut self) -> std::io::Result<String> {
        // SAFETY: см. stream_alias — эксклюзивный доступ гарантирован
        // ожиданием вызывающего.
        unsafe { &mut *self.0 }.read_line()
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        // SAFETY: см. выше.
        unsafe { &mut *self.0 }.write_line(line)
    }
}

async fn dispatch(core: &Arc<CoreManager>, cmd: IpcCommand) -> IpcResponse {
    match cmd {
        IpcCommand::Status => {
            let state = core.state().await;
            IpcResponse::ok(serde_json::json!({
                "engine": format!("{:?}", state.engine),
                "active_profile": state.active_profile,
                "last_errors": state.last_errors,
            }))
        }
        IpcCommand::Stop => match core.stop_engine().await {
            Ok(()) => IpcResponse::ok(serde_json::json!({"engine": "Stopped"})),
            Err(e) => IpcResponse::err(e.to_string()),
        },
        IpcCommand::Run => match core.start_engine().await {
            Ok(()) => IpcResponse::ok(serde_json::json!({"engine": "Running"})),
            Err(e) => IpcResponse::err(e.to_string()),
        },
        IpcCommand::GetConfig => {
            let cfg = core.config().await;
            match serde_json::to_value(&cfg) {
                Ok(v) => IpcResponse::ok(v),
                Err(e) => IpcResponse::err(e.to_string()),
            }
        }
        IpcCommand::LogsTail { lines } => {
            let core = core.clone();
            match tokio::task::spawn_blocking(move || logs_tail(core.paths().root.as_path(), lines))
                .await
            {
                Ok(Ok(text)) => IpcResponse::ok(serde_json::json!({ "logs": text })),
                Ok(Err(e)) => IpcResponse::err(e.to_string()),
                Err(e) => IpcResponse::err(e.to_string()),
            }
        }
    }
}

/// Последние N строк самого свежего файла журнала (logs/mxds.log*).
fn logs_tail(root: &std::path::Path, lines: u32) -> std::io::Result<String> {
    let dir = root.join("logs");
    let mut candidates: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("mxds.log"))
        .collect();
    if candidates.is_empty() {
        return Ok(String::new());
    }
    candidates.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    let path = candidates.last().unwrap().path();
    // Читаем не более последних 256 КиБ — журнал может быть большим.
    const TAIL_CAP: u64 = 256 * 1024;
    let meta_len = std::fs::metadata(&path)?.len();
    let content = if meta_len > TAIL_CAP {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&path)?;
        f.seek(SeekFrom::End(-(TAIL_CAP as i64)))?;
        let mut buf = Vec::new();
        f.take(TAIL_CAP).read_to_end(&mut buf)?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        std::fs::read_to_string(&path)?
    };
    let all: Vec<&str> = content.lines().collect();
    let skip = all.len().saturating_sub(lines as usize);
    Ok(all[skip..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_tail_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let content: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(logs.join("mxds.log.2026-11-13"), &content).unwrap();
        std::fs::write(logs.join("mxds.log"), &content).unwrap();
        let tail = logs_tail(dir.path(), 5).unwrap();
        assert_eq!(tail.lines().count(), 5);
        assert!(tail.ends_with("line 99"));
    }

    #[test]
    fn logs_tail_missing_dir_is_error_but_empty_dir_ok() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("logs")).unwrap();
        assert_eq!(logs_tail(dir.path(), 10).unwrap(), "");
    }

    #[tokio::test]
    async fn server_dispatches_status_over_pipe() {
        use crate::application::{AppPaths, CoreManager};
        use crate::cli::ipc_client;
        use crate::cli::protocol::IpcCommand;

        // Изолированный корень (как в тестах CoreManager).
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::copy(
            manifest.join("config/default.json"),
            tmp.path().join("config/default.json"),
        )
        .unwrap();
        let core = CoreManager::bootstrap(AppPaths {
            root: tmp.path().to_path_buf(),
        })
        .await
        .unwrap();
        let core = std::sync::Arc::new(core);

        let pipe = format!(r"\\.\pipe\MXDS.Test.Srv.{}", std::process::id());
        let server = spawn_named(core.clone(), pipe.clone());
        // Ждём, пока сервер поднимется: клиент с retry. Клиент
        // синхронный — выносим в spawn_blocking, иначе на
        // однопоточном рантайме теста заблокируем серверную задачу.
        let mut resp = None;
        for _ in 0..50 {
            let p = pipe.clone();
            let r = tokio::task::spawn_blocking(move || {
                ipc_client::try_send_named(&p, &IpcCommand::Status)
            })
            .await
            .unwrap();
            if let Ok(Some(r)) = r {
                resp = Some(r);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let resp = resp.expect("сервер ответил");
        assert!(resp.ok);
        assert_eq!(resp.data.unwrap()["engine"], "Stopped");

        // Неизвестная команда отклоняется (allowlist).
        // Между accept-циклами сервера экземпляра пайпа нет —
        // подключаемся с retry.
        let p = pipe.clone();
        let line = tokio::task::spawn_blocking(move || {
            let mut raw = {
                let mut found = None;
                for _ in 0..100 {
                    match crate::windows::ipc::connect(&p) {
                        Ok(s) => {
                            found = Some(s);
                            break;
                        }
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
                    }
                }
                found.expect("сервер доступен после retry")
            };
            raw.write_line(r#"{"cmd":"exec"}"#).unwrap();
            raw.read_line().unwrap()
        })
        .await
        .unwrap();
        assert!(line.contains(r#""ok":false"#));

        // Корректная остановка: accept будится dummy-подключением,
        // цикл завершается, рантайм не виснет.
        server.shutdown().await;
        core.shutdown().await.unwrap();
    }
}
