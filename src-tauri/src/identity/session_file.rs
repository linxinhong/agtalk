//! session.json 读写：每个 agent 的身份文件。

use super::IdentityError;
use crate::paths::{set_permissions_0600, set_permissions_0700};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub address: String,
    pub name: String,
    pub workspace: String,
    pub intro: String,
    pub created_at: String,
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
        };
        write(&dot, "nora", &session).unwrap();
        let read_back = read(&dot, "nora").unwrap();
        assert_eq!(read_back.address, session.address);
        assert_eq!(read_back.name, session.name);
    }
}
