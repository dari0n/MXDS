//! Типы профиля стратегии.
//!
//! Профиль — типизированный эквивалент winws-аргументов оригинала:
//! глобальные фильтры захвата + список правил (между `--new`).

/// Статус совместимости (PROMPT.md Цикл 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CompatStatus {
    Supported,
    PartiallySupported,
    Unsupported,
    #[default]
    NotTested,
}

/// Идентификатор профиля (slug: имя файла без расширения).
pub type ProfileId = String;

/// Метаданные профиля (PROMPT.md Цикл 8).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    pub description: String,
    pub version: String,
    /// Происхождение, например "flowseal-1.10.4" для импортированных.
    pub origin: Option<String>,
}

/// Техника десинхронизации (--dpi-desync=a,b,...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DesyncTechnique {
    Fake,
    Multisplit,
    Multidisorder,
    Syndata,
    HostFakeSplit,
    FakeSplit,
}

/// Позиция сплита (--dpi-desync-split-pos).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SplitPos {
    /// Абсолютная позиция от начала payload.
    Absolute(usize),
    /// Начало SNI-расширения + N (вычисляется движком по пакету).
    SniExtPlus(usize),
    /// Середина домена второго уровня в SNI (вычисляется движком).
    MidSld,
}

/// Приём fooling (--dpi-desync-fooling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FoolingKind {
    Ts,
    BadSeq,
    Md5Sig,
}

/// Типизированная спецификация desync одного правила.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DesyncSpec {
    pub techniques: Vec<DesyncTechnique>,
    /// --dpi-desync-repeats=N (повторы фейка; 1 = один фейк).
    #[serde(default = "one")]
    pub repeats: u32,
    #[serde(default)]
    pub split_pos: Vec<SplitPos>,
    /// --dpi-desync-split-seqovl=N (перекрытие sequence, байт).
    #[serde(default)]
    pub seqovl: Option<u32>,
    /// Файл паттерна для seqovl (bin/...).
    #[serde(default)]
    pub seqovl_pattern: Option<String>,
    #[serde(default)]
    pub fooling: Vec<FoolingKind>,
    /// --dpi-desync-badseq-increment=N (может быть отрицательным).
    #[serde(default)]
    pub badseq_increment: i64,
    /// --dpi-desync-cutoff=nN: прекратить desync после N пакетов соединения.
    #[serde(default)]
    pub cutoff: Option<u32>,
    /// --dpi-desync-any-protocol=1.
    #[serde(default)]
    pub any_protocol: bool,
    /// Фейковые payload'ы по L7-ключу: tls, quic, discord, stun,
    /// unknown, unknown-udp, http. Значение: "bin/file.bin" | "0xHEX" | "!".
    #[serde(default)]
    pub fakes: std::collections::BTreeMap<String, String>,
    /// --dpi-desync-fake-tls-mod (rnd,dupsid,sni=...). Не реализовано
    /// в движке Цикла 7-8 — отражается валидацией.
    #[serde(default)]
    pub fake_tls_mod: Option<String>,
    /// --dpi-desync-hostfakesplit-mod (host=...,altorder=1).
    #[serde(default)]
    pub hostfakesplit_mod: Option<String>,
    /// --dpi-desync-fakedsplit-pattern (0x00...).
    #[serde(default)]
    pub fakedsplit_pattern: Option<String>,
}

fn one() -> u32 {
    1
}

/// Протокол правила.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Protocol {
    Tcp,
    Udp,
}

/// Одно правило движка: фильтр трафика + техника десинхронизации
/// (типизированный эквивалент блока winws-аргументов между `--new`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileRule {
    pub protocol: Protocol,
    /// Порты/диапазоны: "80,443", "19294-19344". Маркеры
    /// %GameFilterTCP%/%GameFilterUDP% сохраняются как "GAME_FILTER".
    pub ports: String,
    /// L7-подсказки (--filter-l7=discord,stun).
    #[serde(default)]
    pub l7: Vec<String>,
    #[serde(default)]
    pub hostlists: Vec<String>,
    /// --hostlist-domains=a,b (встроенный список доменов).
    #[serde(default)]
    pub hostlist_domains: Vec<String>,
    #[serde(default)]
    pub hostlist_exclude: Vec<String>,
    /// --hostlist-exclude-domains=a,b (встроенный список исключений).
    #[serde(default)]
    pub hostlist_exclude_domains: Vec<String>,
    #[serde(default)]
    pub ipsets: Vec<String>,
    #[serde(default)]
    pub ipset_exclude: Vec<String>,
    #[serde(default)]
    pub desync: DesyncSpec,
    /// --ip-id=zero.
    #[serde(default)]
    pub ip_id_zero: bool,
}

/// Профиль стратегии целиком.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub meta: ProfileMeta,
    /// Глобальные фильтры захвата (--wf-tcp / --wf-udp, строки портов).
    pub wf_tcp: String,
    pub wf_udp: String,
    pub rules: Vec<ProfileRule>,
    #[serde(default)]
    pub compat: CompatStatus,
}
