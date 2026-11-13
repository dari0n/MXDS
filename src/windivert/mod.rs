//! WinDivert Adapter — безопасная граница к системному драйверу.
//!
//! FFI (ffi.rs) + safe-адаптер (adapter.rs) реализованы в Цикле 7
//! (docs/WINDIVERT_INTEGRATION.md п.3, п.8).

pub mod adapter;
pub mod errors;
pub mod ffi;

pub use adapter::{default_dll_path, DivertHandle, DivertParam, Packet, MAX_PACKET_SIZE};
pub use errors::DivertError;
pub use ffi::WindivertAddress;
