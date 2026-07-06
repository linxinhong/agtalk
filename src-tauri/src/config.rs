//! 全局配置：~/.config/agtalk2/config.json

use crate::paths::{config_path, ensure_config_dir, set_permissions_0600};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

const DEFAULT_HTTP_PORT: u16 = 19527;
const DEFAULT_HUMAN_NAME: &str = "human";
const DEFAULT_HUMAN_INTRO: &str = "人类收件箱";
const DEFAULT_PREVIEW_CHARS: usize = 4000;
const DEFAULT_INLINE_LIMIT: usize = 2048;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("路径错误: {0}")]
    Paths(#[from] crate::paths::PathsError),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("配置项不存在: {0}")]
    KeyNotFound(String),
    #[error("配置值格式错误: {0}")]
    InvalidValue(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanConfig {
    #[serde(default = "default_human_name")]
    pub name: String,
    #[serde(default = "default_human_intro")]
    pub intro: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageConfig {
    #[serde(default = "default_preview_chars")]
    pub preview_limit_chars: usize,
    #[serde(default = "default_inline_limit")]
    pub inbox_inline_limit_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyPluginEntry {
    pub path: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

fn default_notify_channel() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(default)]
    pub plugins: HashMap<String, NotifyPluginEntry>,
    #[serde(default = "default_notify_channel")]
    pub default: String,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            plugins: HashMap::new(),
            default: default_notify_channel(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgConfig {
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default)]
    pub human: HumanConfig,
    #[serde(default)]
    pub message: MessageConfig,
    #[serde(default)]
    pub notify: NotifyConfig,
}

impl Default for AgConfig {
    fn default() -> Self {
        Self {
            http_port: DEFAULT_HTTP_PORT,
            human: HumanConfig::default(),
            message: MessageConfig::default(),
            notify: NotifyConfig::default(),
        }
    }
}

impl Default for HumanConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_HUMAN_NAME.to_string(),
            intro: DEFAULT_HUMAN_INTRO.to_string(),
        }
    }
}

impl Default for MessageConfig {
    fn default() -> Self {
        Self {
            preview_limit_chars: DEFAULT_PREVIEW_CHARS,
            inbox_inline_limit_bytes: DEFAULT_INLINE_LIMIT,
        }
    }
}

fn default_http_port() -> u16 {
    DEFAULT_HTTP_PORT
}

fn default_human_name() -> String {
    DEFAULT_HUMAN_NAME.to_string()
}

fn default_human_intro() -> String {
    DEFAULT_HUMAN_INTRO.to_string()
}

fn default_preview_chars() -> usize {
    DEFAULT_PREVIEW_CHARS
}

fn default_inline_limit() -> usize {
    DEFAULT_INLINE_LIMIT
}

impl AgConfig {
    /// 从 ~/.config/agtalk2/config.json 加载；不存在则返回默认配置。
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// 保存到 ~/.config/agtalk2/config.json，文件权限 0600。
    pub fn save(&self) -> Result<(), ConfigError> {
        ensure_config_dir()?;
        let path = config_path()?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        set_permissions_0600(&path)?;
        Ok(())
    }

    /// 返回配置文件路径。
    pub fn path() -> Result<std::path::PathBuf, ConfigError> {
        config_path().map_err(ConfigError::Paths)
    }

    /// 读取点分配置项，如 `human.name`。
    pub fn get(&self, key: &str) -> Result<serde_json::Value, ConfigError> {
        let value = serde_json::to_value(self)?;
        let mut current = &value;
        for part in key.split('.') {
            match current.get(part) {
                Some(v) => current = v,
                None => return Err(ConfigError::KeyNotFound(key.to_string())),
            }
        }
        Ok(current.clone())
    }

    /// 设置点分配置项，自动解析整数、布尔与字符串。
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        let parsed = parse_config_value(value);
        let mut config_value = serde_json::to_value(&self)?;
        let parts: Vec<&str> = key.split('.').collect();
        set_nested_value(&mut config_value, &parts, parsed)?;
        *self = serde_json::from_value(config_value)?;
        self.save()?;
        Ok(())
    }
}

fn parse_config_value(s: &str) -> serde_json::Value {
    if s.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(serde_json::Number::from(n));
    }
    serde_json::Value::String(s.to_string())
}

fn set_nested_value(
    value: &mut serde_json::Value,
    parts: &[&str],
    new_val: serde_json::Value,
) -> Result<(), ConfigError> {
    if parts.is_empty() {
        return Err(ConfigError::InvalidValue("空 key".to_string()));
    }
    let mut current = value;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            current[part] = new_val;
            return Ok(());
        }
        if current.get(*part).map(|v| !v.is_object()).unwrap_or(true) {
            current[*part] = serde_json::json!({});
        }
        current = current
            .get_mut(*part)
            .ok_or_else(|| ConfigError::KeyNotFound(parts[..=i].join(".")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os(crate::paths::CONFIG_DIR_ENV);
            std::env::set_var(crate::paths::CONFIG_DIR_ENV, path);
            Self(previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref p) = self.0 {
                std::env::set_var(crate::paths::CONFIG_DIR_ENV, p);
            } else {
                std::env::remove_var(crate::paths::CONFIG_DIR_ENV);
            }
        }
    }

    #[test]
    fn default_config_serializes() {
        let cfg = AgConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("19527"));
    }

    #[test]
    fn get_top_level_and_nested_defaults() {
        let cfg = AgConfig::default();
        assert_eq!(cfg.get("http_port").unwrap(), serde_json::json!(19527));
        assert_eq!(cfg.get("human.name").unwrap(), serde_json::json!("human"));
        assert_eq!(
            cfg.get("message.preview_limit_chars").unwrap(),
            serde_json::json!(4000)
        );
        assert!(cfg.get("no.such.key").is_err());
    }

    #[test]
    fn set_integer_string_and_bool_and_persists() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());

        let mut cfg = AgConfig::load().unwrap();
        cfg.set("http_port", "19528").unwrap();
        cfg.set("human.name", "bob").unwrap();
        cfg.set("notify.default", "none").unwrap();

        let reloaded = AgConfig::load().unwrap();
        assert_eq!(reloaded.http_port, 19528);
        assert_eq!(reloaded.human.name, "bob");
        assert_eq!(reloaded.notify.default, "none");

        assert_eq!(reloaded.get("http_port").unwrap(), serde_json::json!(19528));
    }

    #[test]
    fn path_returns_config_json() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        assert!(AgConfig::path().unwrap().ends_with("config.json"));
    }
}
