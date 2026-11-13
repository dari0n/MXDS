//! FFI-граница WinDivert 2.2 (docs/WINDIVERT_INTEGRATION.md п.3, п.8).
//!
//! DLL загружается динамически (libloading) с проверкой SHA256 перед
//! загрузкой (SECURITY_REVIEW п.25: неподписанная DLL не подменяется
//! молча). Структуры воспроизводят C-layout заголовка windivert.h v2.2;
//! размеры и смещения проверяются compile-time asserts.
//!
//! Весь unsafe WinDivert-слоя сосредоточен здесь и в adapter.rs.

use std::ffi::c_void;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::errors::DivertError;

// --- Зафиксированные хэши дистрибутивных бинарников (SECURITY_REVIEW) ---

/// SHA256 эталонной WinDivert.dll (assets/windivert, zapret-win-bundle).
pub const EXPECTED_DLL_SHA256: &str =
    "c1e060ee19444a259b2162f8af0f3fe8c4428a1c6f694dce20de194ac8d7d9a2";
/// SHA256 эталонного WinDivert64.sys (v2.2, подпись валидна).
pub const EXPECTED_SYS_SHA256: &str =
    "8da085332782708d8767bcace5327a6ec7283c17cfb85e40b03cd2323a90ddc2";

// --- Константы API ---

pub const LAYER_NETWORK: i32 = 0;
pub const PRIORITY_DEFAULT: i16 = 0;

/// Флаги WinDivertOpen.
pub const FLAG_SNIFF: u64 = 1;
pub const FLAG_DROP: u32 = 2;
pub const FLAG_RECV_ONLY: u64 = 4;
pub const FLAG_SEND_ONLY: u64 = 8;

/// Параметры WinDivertSetParam/GetParam.
pub const PARAM_QUEUE_LENGTH: u32 = 0; // 32..=16384 пакетов
pub const PARAM_QUEUE_TIME: u32 = 1; // 100..=16000 мс
                                     // Поставляемая сборка WinDivert (zapret-win-bundle) ожидает QUEUE_SIZE
                                     // в БАЙТАХ (проверено живым прогоном: 65536..=8388608 принимаются,
                                     // значения в КБ — нет), в отличие от документации 2.2 (КБ).
pub const PARAM_QUEUE_SIZE: u32 = 2; // 65536..=8388608 байт
pub const PARAM_VERSION_MAJOR: u32 = 3; // только GetParam
pub const PARAM_VERSION_MINOR: u32 = 4; // только GetParam

pub const QUEUE_LENGTH_MIN: u64 = 32;
pub const QUEUE_LENGTH_MAX: u64 = 16384;
pub const QUEUE_TIME_MIN: u64 = 100;
pub const QUEUE_TIME_MAX: u64 = 16000;
pub const QUEUE_SIZE_BYTES_MIN: u64 = 65536;
pub const QUEUE_SIZE_BYTES_MAX: u64 = 8388608;

/// Режимы WinDivertShutdown.
pub const SHUTDOWN_RECV: u32 = 1;
pub const SHUTDOWN_BOTH: u32 = 3;

// --- Структуры (layout windivert.h v2.2, MSVC-паковка битовых полей) ---

/// WINDIVERT_ADDRESS. Битовые поля C упакованы в `flags`:
/// Layer:8 | Event:8 | Sniffed:1 | Outbound:1 | Loopback:1 | Impostor:1 |
/// IPv6:1 | IPChecksum:1 | TCPChecksum:1 | UDPChecksum:1 | Reserved1:8.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertAddress {
    pub timestamp: i64,
    pub flags: u32,
    pub reserved2: u32,
    /// union Network/Flow/Socket/Reflect; на NETWORK-слое —
    /// Network { IfIdx: u32, SubIfIdx: u32 } в первых 8 байтах.
    pub data: [u8; 40],
}

const _: () = assert!(std::mem::size_of::<WindivertAddress>() == 56);
const _: () = assert!(std::mem::align_of::<WindivertAddress>() == 8);
const _: () = assert!(std::mem::offset_of!(WindivertAddress, flags) == 8);
const _: () = assert!(std::mem::offset_of!(WindivertAddress, data) == 16);

impl WindivertAddress {
    pub fn layer(&self) -> u8 {
        (self.flags & 0xFF) as u8
    }
    pub fn outbound(&self) -> bool {
        self.flags & (1 << 17) != 0
    }
    pub fn loopback(&self) -> bool {
        self.flags & (1 << 18) != 0
    }
    /// Пакет — собственная инъекция WinDivert (подлежит пропуску,
    /// WINDIVERT_INTEGRATION п.7).
    pub fn impostor(&self) -> bool {
        self.flags & (1 << 19) != 0
    }
    pub fn ipv6(&self) -> bool {
        self.flags & (1 << 20) != 0
    }
    /// Interface index (NETWORK-слой).
    pub fn if_idx(&self) -> u32 {
        u32::from_le_bytes(self.data[0..4].try_into().unwrap())
    }
    pub fn sub_if_idx(&self) -> u32 {
        u32::from_le_bytes(self.data[4..8].try_into().unwrap())
    }
}

/// WINDIVERT_IPHDR (20 байт): Version:4 | HdrLength:4 в первом байте
/// (LSB-first по MSVC). Поля многобайтовые — big-endian на проводе.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertIpHdr {
    pub version_hlen: u8,
    pub tos: u8,
    pub length: u16,
    pub id: u16,
    pub frag_off: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_addr: u32,
    pub dst_addr: u32,
}

const _: () = assert!(std::mem::size_of::<WindivertIpHdr>() == 20);

/// WINDIVERT_TCPHDR (20 байт без опций).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertTcpHdr {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    /// Reserved1:4 | HdrLength:4 | Fin..Urg:6 | Reserved2:2 (LSB-first).
    pub flags: u16,
    pub window: u16,
    pub checksum: u16,
    pub urg_ptr: u16,
}

const _: () = assert!(std::mem::size_of::<WindivertTcpHdr>() == 20);

/// WINDIVERT_UDPHDR (8 байт).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertUdpHdr {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

const _: () = assert!(std::mem::size_of::<WindivertUdpHdr>() == 8);

// --- Сигнатуры функций (x64: единая calling convention) ---

type OpenFn = unsafe extern "C" fn(*const i8, i32, i16, u64) -> *mut c_void;
type RecvFn =
    unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *mut u32, *mut WindivertAddress) -> i32;
type SendFn =
    unsafe extern "C" fn(*mut c_void, *const c_void, u32, *mut u32, *const WindivertAddress) -> i32;
type ShutdownFn = unsafe extern "C" fn(*mut c_void, u32) -> i32;
type CloseFn = unsafe extern "C" fn(*mut c_void) -> i32;
type SetParamFn = unsafe extern "C" fn(*mut c_void, u32, u64) -> i32;
type GetParamFn = unsafe extern "C" fn(*mut c_void, u32, *mut u64) -> i32;
type CalcChecksumsFn = unsafe extern "C" fn(*mut c_void, u32, *mut WindivertAddress, u64) -> u32;
type DecrementTtlFn = unsafe extern "C" fn(*mut c_void, u32) -> i32;

/// Загруженный API WinDivert.dll. Владеет Library: символы живы,
/// пока жив экземпляр.
pub struct WinDivertDll {
    // Библиотека должна жить дольше fn-указателей: поле не читается
    // напрямую, но Drop освобождает DLL последним.
    #[allow(dead_code)]
    lib: libloading::Library,
    pub open: OpenFn,
    pub recv: RecvFn,
    pub send: SendFn,
    pub shutdown: ShutdownFn,
    pub close: CloseFn,
    pub set_param: SetParamFn,
    pub get_param: GetParamFn,
    pub calc_checksums: CalcChecksumsFn,
    pub decrement_ttl: DecrementTtlFn,
}

// SAFETY: fn-указатели — plain data; Library потокобезопасна для чтения
// символов. WinDivert API потокобезопасен для разных хендлов; один хендл
// используется одним рабочим потоком движка.
unsafe impl Send for WinDivertDll {}
unsafe impl Sync for WinDivertDll {}

/// SHA256 файла в hex.
pub fn sha256_file(path: &Path) -> Result<String, DivertError> {
    let data = std::fs::read(path).map_err(|_| DivertError::DriverNotFound)?;
    Ok(hex_lower(&Sha256::digest(&data)))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl WinDivertDll {
    /// Загрузить WinDivert.dll по явному пути с проверкой SHA256
    /// (и рядом лежащего WinDivert64.sys). Расхождение хэша —
    /// отказ загрузки (SECURITY_REVIEW п.25).
    pub fn load(dll_path: &Path) -> Result<Self, DivertError> {
        let dll_hash = sha256_file(dll_path)?;
        if !dll_hash.eq_ignore_ascii_case(EXPECTED_DLL_SHA256) {
            tracing::error!(path = ?dll_path, hash = %dll_hash, "WinDivert.dll: SHA256 не совпадает");
            return Err(DivertError::DriverSignatureInvalid);
        }
        let sys_path = dll_path.with_file_name("WinDivert64.sys");
        let sys_hash = sha256_file(&sys_path)?;
        if !sys_hash.eq_ignore_ascii_case(EXPECTED_SYS_SHA256) {
            tracing::error!(path = ?sys_path, hash = %sys_hash, "WinDivert64.sys: SHA256 не совпадает");
            return Err(DivertError::DriverSignatureInvalid);
        }

        // SAFETY: путь проверен, хэш совпадает с эталоном дистрибутива.
        let lib = unsafe { libloading::Library::new(dll_path) }
            .map_err(|_| DivertError::DriverNotFound)?;

        // SAFETY: каждый символ — экспорт WinDivert 2.2 с сигнатурой из
        // windivert.h; fn-указатель копируется из Symbol и валиден, пока
        // жив `lib` (хранится в этой же структуре).
        unsafe {
            Ok(Self {
                open: *lib
                    .get::<OpenFn>(b"WinDivertOpen\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                recv: *lib
                    .get::<RecvFn>(b"WinDivertRecv\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                send: *lib
                    .get::<SendFn>(b"WinDivertSend\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                shutdown: *lib
                    .get::<ShutdownFn>(b"WinDivertShutdown\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                close: *lib
                    .get::<CloseFn>(b"WinDivertClose\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                set_param: *lib
                    .get::<SetParamFn>(b"WinDivertSetParam\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                get_param: *lib
                    .get::<GetParamFn>(b"WinDivertGetParam\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                calc_checksums: *lib
                    .get::<CalcChecksumsFn>(b"WinDivertHelperCalcChecksums\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                decrement_ttl: *lib
                    .get::<DecrementTtlFn>(b"WinDivertHelperDecrementTTL\0")
                    .map_err(|_| DivertError::Unknown(u32::MAX))?,
                lib,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_layout_flags() {
        let mut a = WindivertAddress {
            timestamp: 0,
            flags: 0,
            reserved2: 0,
            data: [0; 40],
        };
        assert!(!a.outbound());
        a.flags |= 1 << 17;
        assert!(a.outbound());
        a.flags |= 1 << 19;
        assert!(a.impostor());
        assert_eq!(a.layer(), 0); // layer NETWORK = 0 (биты 0..8 не выставлены)
    }

    #[test]
    fn sha256_of_known_payloads() {
        // Проверка хэш-функции на эталонных файлах дистрибутива.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/windivert");
        assert_eq!(
            sha256_file(&dir.join("WinDivert.dll")).unwrap(),
            EXPECTED_DLL_SHA256
        );
        assert_eq!(
            sha256_file(&dir.join("WinDivert64.sys")).unwrap(),
            EXPECTED_SYS_SHA256
        );
    }

    #[test]
    fn dll_loads_and_exports_resolve() {
        // Загрузка DLL не требует прав администратора (открытие хендла —
        // требует; проверяется в интеграционном тесте адаптера).
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/windivert");
        let dll = WinDivertDll::load(&dir.join("WinDivert.dll"));
        assert!(dll.is_ok(), "загрузка DLL: {:?}", dll.err());
    }

    #[test]
    fn dll_hash_mismatch_rejected() {
        // Подменённая DLL (другой файл с тем же именем) отклоняется.
        let tmp = std::env::temp_dir().join(format!("mxds_test_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let fake_dll = tmp.join("WinDivert.dll");
        let fake_sys = tmp.join("WinDivert64.sys");
        std::fs::write(&fake_dll, b"not a real dll").unwrap();
        std::fs::write(&fake_sys, b"not a real sys").unwrap();
        let r = WinDivertDll::load(&fake_dll);
        assert!(matches!(r, Err(DivertError::DriverSignatureInvalid)));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
