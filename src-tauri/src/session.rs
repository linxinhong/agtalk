//! sessions/<name>.json 读写（每个 Agent 的 session 凭证）。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

pub const SESSION_FILE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    pub name: String,
    pub session: SessionMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub token: String,
    #[serde(default = "default_active")]
    pub status: String,
    pub created_at: String,
}

fn default_active() -> String {
    "active".into()
}

pub fn read_session(name: &str) -> Result<Option<SessionFile>> {
    let path = paths::session_json_path(name)?;
    if !path.exists() {
        return Ok(None);
    }
    let data =
        std::fs::read_to_string(&path).with_context(|| format!("读取 session 失败: {:?}", path))?;
    let sf: SessionFile = serde_json::from_str(&data).context("解析 session 失败")?;
    Ok(Some(sf))
}

pub fn write_session(name: &str, sf: &mut SessionFile) -> Result<()> {
    let path = paths::session_json_path(name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    sf.version = SESSION_FILE_VERSION;
    let data = serde_json::to_string_pretty(sf)?;
    std::fs::write(&path, data)?;
    set_permissions_0600(&path);
    Ok(())
}

#[allow(dead_code)]
pub fn remove_session(name: &str) -> Result<()> {
    let path = paths::session_json_path(name)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// 列出所有 active session 的名字
pub fn list_active_sessions() -> Result<Vec<String>> {
    let dir = paths::sessions_dir()?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Ok(Some(sf)) = read_session(&name) {
            if sf.session.status == "active" {
                names.push(name);
            }
        }
    }
    Ok(names)
}

#[cfg(unix)]
fn set_permissions_0600(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_permissions_0600(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::session_json_path;
    use std::io::Write;

    #[test]
    fn test_remove_session_file() {
        let tmp = tempfile::tempdir().unwrap();
        let agtalk_dir = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&agtalk_dir).unwrap();
        std::env::set_var("AGTALK_ROOT", agtalk_dir.parent().unwrap());

        let name = "purge-test-agent";
        let path = session_json_path(name).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"{}").unwrap();
        drop(f);
        assert!(path.exists());

        remove_session(name).unwrap();
        assert!(!path.exists());

        // 重复删除不应报错
        remove_session(name).unwrap();
    }
}
