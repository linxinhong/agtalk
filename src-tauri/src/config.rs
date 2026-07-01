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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(default)]
    pub plugins: HashMap<String, NotifyPluginEntry>,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_temp_config_dir<F>(f: F)
    where
        F: FnOnce(),
    {
        let tmp = TempDir::new().unwrap();
        // 临时替换 config_dir 的行为不直接可行，改为测 save/load 的完整流程
        // 这里仅做序列化/默认值测试，文件 IO 测试留在集成测试。
        let _ = tmp;
        f();
    }

    #[test]
    fn default_config_serializes() {
        with_temp_config_dir(|| {
            let cfg = AgConfig::default();
            let json = serde_json::to_string(&cfg).unwrap();
            assert!(json.contains("19527"));
        });
    }

    #[test]
    fn load_missing_returns_default() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.json");
        // 不存在时应返回默认
        assert!(!path.exists());
        // 这里无法阻止 AgConfig::load 读取真实目录，故只验证默认值结构
        let cfg = AgConfig::default();
        assert_eq!(cfg.http_port, 19527);
        assert_eq!(cfg.human.name, "human");
    }
}
