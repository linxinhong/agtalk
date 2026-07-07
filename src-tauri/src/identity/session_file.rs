//! session.json 读写：每个 agent 的身份文件。

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFile {
    pub address: String,
    pub name: String,
    pub workspace: String,
    pub intro: String,
    pub created_at: String,
    /// 注册时当前进程命令（argv 拼接），方便调试与定位。
    #[serde(default)]
    pub command: String,
    /// 持久化 notify 通道，如 "zellij" / "tmux" / "none" / "auto"。
    #[serde(default)]
    pub notify_channel: String,
    /// zellij/tmux 定位信息。
    #[serde(default)]
    pub notify_target: NotifyTarget,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_session_file() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端 review".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            command: "agtalk".to_string(),
            notify_channel: "auto".to_string(),
            notify_target: NotifyTarget::Plugin {
                name: "zellij".to_string(),
                endpoint: serde_json::json!({ "session": "sess", "pane": "1" }),
            },
        };
        write(&dot, "nora", &session).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.address, session.address);
        assert_eq!(read_back.name, session.name);
        assert_eq!(read_back.command, session.command);
        assert_eq!(read_back.notify_channel, session.notify_channel);
        assert_eq!(read_back.notify_target, session.notify_target);
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
        assert_eq!(read_back.command, "");
        assert_eq!(read_back.notify_channel, "");
        assert_eq!(read_back.notify_target, NotifyTarget::None);
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
        assert_eq!(read_back.notify_channel, "zellij");
        assert_eq!(
            read_back.notify_target,
            NotifyTarget::Plugin {
                name: "zellij".to_string(),
                endpoint: serde_json::json!({"session": "agtalk", "pane": "4"}),
            }
        );
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
        assert_eq!(read_back.notify_channel, "tmux");
        assert_eq!(
            read_back.notify_target,
            NotifyTarget::Plugin {
                name: "tmux".to_string(),
                endpoint: serde_json::json!({"session": "agtalk", "pane": "4"}),
            }
        );
    }
}
