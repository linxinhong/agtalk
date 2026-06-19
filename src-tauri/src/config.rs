//! 全局配置：~/.config/agtalk/config.json
//!
//! 阈值与附件目录均可配置，daemon 启动时加载一次。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::paths;

const DEFAULT_INLINE_LIMIT: usize = 2048;
const DEFAULT_PREVIEW_CHARS: usize = 600;
const DEFAULT_ATTACHMENT_THRESHOLD: usize = 8192;
const DEFAULT_HARD_FILE_THRESHOLD: usize = 262144;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageConfig {
    #[serde(default = "default_inline_limit")]
    pub inbox_inline_limit_bytes: usize,
    #[serde(default = "default_preview_chars")]
    pub preview_limit_chars: usize,
    #[serde(default = "default_attachment_threshold")]
    pub attachment_threshold_bytes: usize,
    #[serde(default = "default_hard_file_threshold")]
    pub hard_file_threshold_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_attachment_dir")]
    pub attachment_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(default = "default_send_enter")]
    pub default_send_enter: bool,
    #[serde(default = "default_notify_plugins")]
    pub plugins: HashMap<String, NotifyPluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyPluginEntry {
    Builtin,
    Command {
        path: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgConfig {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub message: MessageConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub notify: NotifyConfig,
}

fn default_inline_limit() -> usize {
    DEFAULT_INLINE_LIMIT
}
fn default_preview_chars() -> usize {
    DEFAULT_PREVIEW_CHARS
}
fn default_attachment_threshold() -> usize {
    DEFAULT_ATTACHMENT_THRESHOLD
}
fn default_hard_file_threshold() -> usize {
    DEFAULT_HARD_FILE_THRESHOLD
}
fn default_send_enter() -> bool {
    true
}
fn default_notify_plugins() -> HashMap<String, NotifyPluginEntry> {
    let mut m = HashMap::new();
    m.insert("zellij".to_string(), NotifyPluginEntry::Builtin);
    m.insert("tmux".to_string(), NotifyPluginEntry::Builtin);
    m
}

fn default_attachment_dir() -> String {
    paths::config_dir()
        .join("attachments")
        .to_string_lossy()
        .to_string()
}

impl Default for MessageConfig {
    fn default() -> Self {
        Self {
            inbox_inline_limit_bytes: DEFAULT_INLINE_LIMIT,
            preview_limit_chars: DEFAULT_PREVIEW_CHARS,
            attachment_threshold_bytes: DEFAULT_ATTACHMENT_THRESHOLD,
            hard_file_threshold_bytes: DEFAULT_HARD_FILE_THRESHOLD,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            attachment_dir: default_attachment_dir(),
        }
    }
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            default_send_enter: true,
            plugins: default_notify_plugins(),
        }
    }
}

impl Default for AgConfig {
    fn default() -> Self {
        Self {
            version: 1,
            message: MessageConfig::default(),
            storage: StorageConfig::default(),
            notify: NotifyConfig::default(),
        }
    }
}

impl AgConfig {
    pub fn load() -> Result<Self> {
        let path = paths::config_json_path();
        if !path.exists() {
            let cfg = Self::default();
            cfg.save()?;
            return Ok(cfg);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("无法读取配置文件: {:?}", path))?;
        let mut cfg: AgConfig = serde_json::from_str(&content)
            .with_context(|| format!("配置文件 JSON 解析失败: {:?}", path))?;
        // 缺省字段兜底
        if cfg.version == 0 {
            cfg.version = 1;
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_json_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建配置目录: {:?}", parent))?;
        }
        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        std::fs::write(&path, content).with_context(|| format!("无法写入配置文件: {:?}", path))?;
        Ok(())
    }

    /// 解析带 ~ 的附件目录为绝对路径
    pub fn attachment_dir(&self) -> Result<PathBuf> {
        expand_tilde(&self.storage.attachment_dir)
    }

    /// 点号分隔读取，如 "message.attachment_threshold_bytes"
    pub fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let value = serde_json::to_value(self).context("序列化配置失败")?;
        let mut current = &value;
        for part in key.split('.') {
            if part.is_empty() {
                continue;
            }
            current = current
                .get(part)
                .ok_or_else(|| anyhow::anyhow!("配置项不存在: {}", key))?;
        }
        Ok(Some(current.clone()))
    }

    /// 点号分隔写入，如 "message.attachment_threshold_bytes"
    pub fn set(&mut self, key: &str, value_str: &str) -> Result<()> {
        let mut value = serde_json::to_value(&*self).context("序列化配置失败")?;
        let parts: Vec<&str> = key.split('.').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            anyhow::bail!("配置 key 不能为空");
        }

        let mut current = &mut value;
        for part in &parts[..parts.len() - 1] {
            current = current
                .get_mut(*part)
                .ok_or_else(|| anyhow::anyhow!("配置项不存在: {}", key))?;
        }

        let last = parts.last().unwrap();
        let target = current
            .get_mut(*last)
            .ok_or_else(|| anyhow::anyhow!("配置项不存在: {}", key))?;

        // 根据现有类型转换 value
        let new_value: serde_json::Value = match target {
            serde_json::Value::Number(_) | serde_json::Value::Null => {
                if let Ok(n) = value_str.parse::<u64>() {
                    serde_json::Value::Number(n.into())
                } else if let Ok(n) = value_str.parse::<i64>() {
                    serde_json::Value::Number(n.into())
                } else if let Ok(f) = value_str.parse::<f64>() {
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or_else(|| serde_json::Value::String(value_str.to_string()))
                } else {
                    serde_json::Value::String(value_str.to_string())
                }
            }
            serde_json::Value::Bool(_) => serde_json::Value::Bool(value_str == "true"),
            _ => serde_json::Value::String(value_str.to_string()),
        };

        *target = new_value;
        *self = serde_json::from_value(value).context("反序列化配置失败")?;
        Ok(())
    }
}

fn expand_tilde(path: &str) -> Result<PathBuf> {
    if path.starts_with("~/") || path == "~" {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("无法获取用户主目录"))?;
        let rest = if path == "~" { "" } else { &path[2..] };
        Ok(home.join(rest))
    } else {
        Ok(PathBuf::from(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default_values() {
        let cfg = AgConfig::default();
        assert_eq!(cfg.message.inbox_inline_limit_bytes, 2048);
        assert_eq!(cfg.message.preview_limit_chars, 600);
        assert_eq!(cfg.message.attachment_threshold_bytes, 8192);
        assert_eq!(cfg.message.hard_file_threshold_bytes, 262144);
    }

    #[test]
    fn test_config_get_set() {
        let mut cfg = AgConfig::default();
        assert_eq!(
            cfg.get("message.attachment_threshold_bytes")
                .unwrap()
                .unwrap(),
            serde_json::Value::Number(8192u64.into())
        );
        cfg.set("message.attachment_threshold_bytes", "4096")
            .unwrap();
        assert_eq!(cfg.message.attachment_threshold_bytes, 4096);
    }
}
