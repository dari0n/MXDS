//! Типы конфигурации приложения.

use crate::engine::EngineConfig;
use crate::logging::LogLevel;

/// Режим Game Filter (оригинал: utils/game_filter.enabled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum GameFilterMode {
    #[default]
    Disabled,
    All,
    Tcp,
    Udp,
}

/// Game Filter: режим + валидированные диапазоны портов
/// (валидация 1-65535, списки и диапазоны через запятую — Цикл 5,
/// логика перенесена из validate_game_filter_range оригинала).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameFilter {
    pub mode: GameFilterMode,
    pub tcp_ports: String,
    pub udp_ports: String,
}

impl Default for GameFilter {
    fn default() -> Self {
        Self {
            mode: GameFilterMode::Disabled,
            tcp_ports: "1024-65535".to_string(),
            udp_ports: "1024-65535".to_string(),
        }
    }
}

/// Режим IPSet-фильтра (оригинал кодирует содержимым ipset-all.txt;
/// MXDS хранит явно — FUNCTIONALITY_MAP F-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum IpsetMode {
    None,
    Any,
    #[default]
    Loaded,
}

/// Корневая конфигурация приложения.
/// Приоритет: значения по умолчанию < config.json < ENV (PROMPT.md Цикл 5).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub log_level: LogLevel,
    pub active_profile: Option<String>,
    #[serde(default)]
    pub game_filter: GameFilter,
    #[serde(default)]
    pub ipset_mode: IpsetMode,
    #[serde(default)]
    pub check_updates: bool,
    /// Автоподбор профиля при запуске движка: проверить активный профиль
    /// доступностью YouTube и, если не работает, перебрать остальные
    /// (приоритет — отмеченным рабочими). Результаты отмечаются
    /// в data/profile-compat.json.
    #[serde(default = "default_true")]
    pub auto_select: bool,
    #[serde(default)]
    pub engine: EngineConfig,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: crate::CONFIG_SCHEMA_VERSION,
            log_level: LogLevel::Info,
            active_profile: None,
            game_filter: GameFilter::default(),
            ipset_mode: IpsetMode::Loaded,
            check_updates: false,
            auto_select: true,
            engine: EngineConfig::default(),
        }
    }
}

/// Централизованный менеджер конфигурации (PROMPT.md Цикл 5).
/// Единственная точка чтения/записи настроек приложения.
#[derive(Debug)]
pub struct ConfigManager {
    config: Config,
    paths: crate::config::loader::ConfigPaths,
    /// Предупреждения загрузки (деградации, восстановление из .bak).
    pub load_warnings: Vec<String>,
}

impl ConfigManager {
    /// Загрузка default.json → config.json → ENV; миграция схемы при
    /// необходимости (с сохранением мигрированной версии).
    pub fn load(
        paths: crate::config::loader::ConfigPaths,
    ) -> Result<Self, crate::config::ConfigError> {
        let outcome = crate::config::loader::load(&paths)?;
        let mut config = outcome.config;
        if config.schema_version < crate::CONFIG_SCHEMA_VERSION {
            config = crate::config::repository::migrate(config)?;
            crate::config::repository::save(&paths.config_path, &config)?;
        }
        Ok(Self {
            config,
            paths,
            load_warnings: outcome.warnings,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Валидация текущей конфигурации.
    pub fn validate(&self) -> Vec<crate::strategy::validation::ValidationIssue> {
        crate::config::validation::validate(&self.config)
    }

    /// Контролируемое применение изменений: валидация до записи,
    /// невалидная конфигурация отклоняется без побочных эффектов.
    pub fn update(
        &mut self,
        f: impl FnOnce(&mut Config),
    ) -> Result<Vec<crate::strategy::validation::ValidationIssue>, crate::config::ConfigError> {
        let mut candidate = self.config.clone();
        f(&mut candidate);
        let issues = crate::config::validation::validate(&candidate);
        if issues
            .iter()
            .any(|i| i.severity == crate::strategy::validation::IssueSeverity::Error)
        {
            return Ok(issues); // не применяем; ошибки возвращаются вызывающему
        }
        crate::config::repository::save(&self.paths.config_path, &candidate)?;
        self.config = candidate;
        tracing::info!("конфигурация обновлена и сохранена");
        Ok(issues)
    }

    /// Прямое сохранение текущего состояния.
    pub fn save(&self) -> Result<(), crate::config::ConfigError> {
        crate::config::repository::save(&self.paths.config_path, &self.config)
    }
}
