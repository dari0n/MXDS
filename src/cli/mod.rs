//! CLI (clap) — Цикл 12.
//! Команды: status, check, diagnostics, config show/validate,
//! profiles list/info, run, stop, version (PROMPT.md Цикл 12).
//! Коды завершения: docs/ERROR_HANDLING.md п.6.
//!
//! CLI не содержит бизнес-логики: только вызовы Application Core
//! (при работающем экземпляре — через Named Pipe IPC, D-009).

pub mod commands;
pub mod ipc_client;
pub mod ipc_server;
pub mod protocol;
