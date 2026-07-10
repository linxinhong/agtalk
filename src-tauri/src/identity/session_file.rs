//! session.json 读写：每个 agent 的身份文件。
//!
//! v2 schema：
//! {
//!   "version": 2,
//!   "address": "...",
//!   "name": "...",
//!   "intro": "...",
//!   "created_at": "...",
//!   "registered_by": "...",   // 可选
//!   "notify": {
//!     "channel": "plugin:zellij",
//!     "endpoint": { "session": "...", "pane": "..." }
//!   }
//! }

use super::IdentityError;
use crate::paths::{set_permissions_0600, set_permissions_0700};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};

/// notify 通道定位信息，用于精准注入提示。
///
/// v2 只保留 `None` 与 `Plugin`：zellij/tmux 已迁出 core，由外部 notify plugin 通过
/// `discover` 提供 endpoint，agtalk core 只负责透传。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NotifyTarget {
    #[default]
    None,
    Plugin {
        name: String,
        /// 插件自定义的 endpoint 对象，由 plugin discover 输出、send 接收。
        endpoint: serde_json::Value,
    },
}

impl NotifyTarget {
    /// 如果是 Plugin 变体，返回插件名；否则返回 None。
    pub fn plugin_name(&self) -> Option<&str> {
        match self {
            NotifyTarget::Plugin { name, .. } => Some(name),
            _ => None,
        }
    }
}

impl Serialize for NotifyTarget {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = match self {
            NotifyTarget::None => serde_json::json!({"type": "none"}),
            NotifyTarget::Plugin { name, endpoint } => {
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "type".to_string(),
                    serde_json::Value::String("plugin".to_string()),
                );
                obj.insert("name".to_string(), serde_json::Value::String(name.clone()));
                obj.insert("endpoint".to_string(), endpoint.clone());
                serde_json::Value::Object(obj)
            }
        };
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NotifyTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(serde::de::Error::custom)
    }
}

impl NotifyTarget {
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        let Some(obj) = value.as_object() else {
            return Ok(NotifyTarget::None);
        };
        let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("none");
        match ty {
            "plugin" => {
                let name = obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let endpoint = obj.get("endpoint").cloned().unwrap_or_default();
                Ok(NotifyTarget::Plugin { name, endpoint })
            }
            "zellij" | "tmux" => {
                // 旧版 session 兼容：把 zellij/tmux 映射为 plugin:<name>。
                let name = ty.to_string();
                let endpoint = serde_json::json!({
                    "session": obj.get("session").and_then(|v| v.as_str()).unwrap_or(""),
                    "pane": obj.get("pane").and_then(|v| v.as_str()).unwrap_or(""),
                });
                Ok(NotifyTarget::Plugin { name, endpoint })
            }
            _ => Ok(NotifyTarget::None),
        }
    }
}

/// v2 session.json 中的 notify 对象。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionNotify {
    pub channel: String,
    #[serde(default)]
    pub endpoint: serde_json::Value,
}

impl SessionNotify {
    pub fn none() -> Self {
        Self {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        }
    }

    pub fn plugin(name: &str, endpoint: serde_json::Value) -> Self {
        Self {
            channel: format!("plugin:{}", name),
            endpoint,
        }
    }

    pub fn to_notify_target(&self) -> NotifyTarget {
        if self.channel.eq_ignore_ascii_case("none") || self.channel.is_empty() {
            return NotifyTarget::None;
        }
        if let Some(name) = self.channel.strip_prefix("plugin:") {
            NotifyTarget::Plugin {
                name: name.to_string(),
                endpoint: self.endpoint.clone(),
            }
        } else {
            // 旧通道名如 "zellij" / "tmux" 直接映射。
            NotifyTarget::Plugin {
                name: self.channel.clone(),
                endpoint: self.endpoint.clone(),
            }
        }
    }
}

/// v2 session.json 内容。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionFile {
    pub version: u32,
    pub address: String,
    pub name: String,
    pub intro: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_by: Option<String>,
    pub notify: SessionNotify,
}

impl SessionFile {
    pub fn new(address: String, name: String, intro: String) -> Self {
        Self {
            version: 2,
            address,
            name,
            intro,
            created_at: iso_now(),
            registered_by: None,
            notify: SessionNotify::none(),
        }
    }

    /// 将 v2 notify 对象转成 notify 模块内部使用的 NotifyTarget。
    pub fn notify_target(&self) -> NotifyTarget {
        self.notify.to_notify_target()
    }

    /// 返回 channel 摘要，兼容旧 notify_channel 语义。
    pub fn notify_channel(&self) -> String {
        self.notify.channel.clone()
    }
}

impl<'de> Deserialize<'de> for SessionFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let obj = value
            .as_object_mut()
            .ok_or_else(|| serde::de::Error::custom("session.json 不是对象"))?;

        let version = obj.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

        let address = obj
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let intro = obj
            .get("intro")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let created_at = obj
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or(&iso_now())
            .to_string();

        let registered_by = obj
            .get("registered_by")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                obj.get("command")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            });

        let notify = if version >= 2 {
            obj.get("notify")
                .cloned()
                .and_then(|v| serde_json::from_value::<SessionNotify>(v).ok())
                .unwrap_or_default()
        } else {
            // v1 兼容：读取 notify_channel + notify_target
            let notify_channel = obj
                .get("notify_channel")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_string();
            let notify_target = obj
                .get("notify_target")
                .cloned()
                .and_then(|v| NotifyTarget::from_value(v).ok())
                .unwrap_or_default();
            let channel = if notify_channel.is_empty() {
                "none".to_string()
            } else if notify_channel.eq_ignore_ascii_case("zellij")
                || notify_channel.eq_ignore_ascii_case("tmux")
            {
                format!("plugin:{}", notify_channel.to_lowercase())
            } else {
                notify_channel
            };
            SessionNotify {
                channel,
                endpoint: match &notify_target {
                    NotifyTarget::None => serde_json::Value::Null,
                    NotifyTarget::Plugin { endpoint, .. } => endpoint.clone(),
                },
            }
        };

        Ok(SessionFile {
            version: 2,
            address,
            name,
            intro,
            created_at,
            registered_by,
            notify,
        })
    }
}

fn session_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    dot_agtalk.join(name).join("session.json")
}

/// 读取 session.json
pub fn read(dot_agtalk: &Path, name: &str) -> Result<SessionFile, IdentityError> {
    let path = session_path(dot_agtalk, name);
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&content)?)
}

/// 写入 session.json，目录权限 0700，文件权限 0600。
pub fn write(
    dot_agtalk: &Path,
    name: &str,
    session: &SessionFile,
) -> Result<PathBuf, IdentityError> {
    let dir = dot_agtalk.join(name);
    std::fs::create_dir_all(&dir)?;
    set_permissions_0700(&dir)?;

    let path = dir.join("session.json");
    let content = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;
    Ok(path)
}

/// 删除 session 文件及其目录
pub fn remove(dot_agtalk: &Path, name: &str) -> Result<(), IdentityError> {
    let dir = dot_agtalk.join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// 列出 `.agtalk/` 下所有包含 `session.json` 的 agent name。
pub fn list_session_names(dot_agtalk: &Path) -> Result<Vec<String>, IdentityError> {
    let mut names = Vec::new();
    if !dot_agtalk.exists() {
        return Ok(names);
    }
    for entry in std::fs::read_dir(dot_agtalk)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("session.json").is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_session_file() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            version: 2,
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            intro: "前端 review".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            registered_by: Some("agtalk".to_string()),
            notify: SessionNotify::plugin(
                "zellij",
                serde_json::json!({ "session": "sess", "pane": "1" }),
            ),
        };
        write(&dot, "nora", &session).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.version, 2);
        assert_eq!(read_back.address, session.address);
        assert_eq!(read_back.name, session.name);
        assert_eq!(read_back.intro, session.intro);
        assert_eq!(read_back.registered_by, session.registered_by);
        assert_eq!(read_back.notify, session.notify);
        assert_eq!(read_back.notify_channel(), "plugin:zellij");
        assert!(matches!(
            read_back.notify_target(),
            NotifyTarget::Plugin { name, .. } if name == "zellij"
        ));
    }

    #[test]
    fn old_session_file_defaults() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot.join("nora")).unwrap();
        let old = r#"{
            "address": "550e8400-e29b-41d4-a716-446655440000",
            "name": "nora",
            "workspace": "projA",
            "intro": "前端 review",
            "created_at": "2026-07-01T00:00:00Z"
        }"#;
        std::fs::write(dot.join("nora").join("session.json"), old).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.version, 2);
        assert_eq!(read_back.notify.channel, "none");
        assert_eq!(read_back.notify.endpoint, serde_json::Value::Null);
    }

    #[test]
    fn legacy_command_migrates_to_registered_by() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot.join("nora")).unwrap();
        let old = r#"{
            "address": "550e8400-e29b-41d4-a716-446655440000",
            "name": "nora",
            "command": "/Users/me/.local/bin/agtalk",
            "intro": "前端 review",
            "created_at": "2026-07-01T00:00:00Z"
        }"#;
        std::fs::write(dot.join("nora").join("session.json"), old).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(
            read_back.registered_by,
            Some("/Users/me/.local/bin/agtalk".to_string())
        );
    }

    #[test]
    fn legacy_zellij_session_maps_to_plugin() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot.join("nora")).unwrap();
        let old = r#"{
            "address": "550e8400-e29b-41d4-a716-446655440000",
            "name": "nora",
            "workspace": "projA",
            "intro": "前端 review",
            "created_at": "2026-07-01T00:00:00Z",
            "command": "/Users/me/.local/bin/agtalk",
            "notify_channel": "zellij",
            "notify_target": {
                "type": "zellij",
                "session": "agtalk",
                "pane": "4"
            }
        }"#;
        std::fs::write(dot.join("nora").join("session.json"), old).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.notify.channel, "plugin:zellij");
        assert_eq!(
            read_back.notify.endpoint,
            serde_json::json!({"session": "agtalk", "pane": "4"})
        );
        assert!(matches!(
            read_back.notify_target(),
            NotifyTarget::Plugin { name, .. } if name == "zellij"
        ));
    }

    #[test]
    fn legacy_tmux_session_maps_to_plugin() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot.join("nora")).unwrap();
        let old = r#"{
            "address": "550e8400-e29b-41d4-a716-446655440000",
            "name": "nora",
            "workspace": "projA",
            "intro": "前端 review",
            "created_at": "2026-07-01T00:00:00Z",
            "notify_channel": "tmux",
            "notify_target": {
                "type": "tmux",
                "session": "agtalk",
                "pane": "4"
            }
        }"#;
        std::fs::write(dot.join("nora").join("session.json"), old).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.notify.channel, "plugin:tmux");
        assert_eq!(
            read_back.notify.endpoint,
            serde_json::json!({"session": "agtalk", "pane": "4"})
        );
    }

    #[test]
    fn v2_session_reads_notify_object() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot.join("nora")).unwrap();
        let v2 = r#"{
            "version": 2,
            "address": "550e8400-e29b-41d4-a716-446655440000",
            "name": "nora",
            "intro": "前端 review",
            "created_at": "2026-07-01T00:00:00Z",
            "notify": {
                "channel": "plugin:zellij",
                "endpoint": {"session": "agtalk", "pane": "5"}
            }
        }"#;
        std::fs::write(dot.join("nora").join("session.json"), v2).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.notify.channel, "plugin:zellij");
        assert_eq!(
            read_back.notify.endpoint,
            serde_json::json!({"session": "agtalk", "pane": "5"})
        );
    }

    #[test]
    fn write_outputs_v2_schema() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile::new(
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "nora".to_string(),
            "前端 review".to_string(),
        );
        write(&dot, "nora", &session).unwrap();
        let content = std::fs::read_to_string(dot.join("nora").join("session.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(json["version"], 2);
        assert!(json.get("workspace").is_none());
        assert!(json.get("command").is_none());
        assert!(json.get("notify_channel").is_none());
        assert!(json.get("notify_target").is_none());
        assert!(json["notify"].is_object());
        assert_eq!(json["notify"]["channel"], "none");
    }
}
