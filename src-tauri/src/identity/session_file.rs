//! session.json 读写：每个 agent 的身份文件。

use super::IdentityError;
use crate::paths::{set_permissions_0600, set_permissions_0700};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 终端/多路复用器/插件定位信息，用于 notify 通道精准注入提示。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotifyTarget {
    #[default]
    None,
    Zellij {
        session: String,
        pane: String,
    },
    Tmux {
        pane: String,
    },
    Plugin {
        name: String,
    },
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
            notify_target: NotifyTarget::Zellij {
                session: "sess".to_string(),
                pane: "1".to_string(),
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
}
