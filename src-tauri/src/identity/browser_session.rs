//! 浏览器扩展会话：token 认证 + session.json。
//!
//! 浏览器扩展无法访问本地 `.agtalk/` 文件系统，因此 daemon 在全局配置目录下
//! 为其维护独立的 workspace：`~/.config/agtalk2/browser/<name>/session.json`。

use super::{mailbox, session_file, IdentityError};
use crate::identity::auth::AuthenticatedSession;
use crate::paths::{ensure_browser_workspace_dir, set_permissions_0600, set_permissions_0700};
use crate::storage::Storage;
use rusqlite::params;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn browser_workspace() -> Result<PathBuf, IdentityError> {
    Ok(ensure_browser_workspace_dir()?)
}

fn session_dir(workspace: &Path, name: &str) -> PathBuf {
    workspace.join(name)
}

/// 创建浏览器扩展身份。
/// 返回 (address, name, token)。
pub fn create(
    storage: &Storage,
    name: Option<String>,
    intro: Option<String>,
    workspace_name: Option<String>,
) -> Result<(String, String, String), IdentityError> {
    let name = name.unwrap_or_else(|| format!("browser-{}", short_id()));
    let intro = intro.unwrap_or_default();
    let workspace = workspace_name.unwrap_or_default();

    let address = mailbox::create(storage, &name, &intro, &workspace)?;
    let token = Uuid::new_v4().to_string();

    {
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO browser_sessions (address, token, name) VALUES (?1, ?2, ?3)",
            params![&address, &token, &name],
        )?;
    }

    let dot_browser = browser_workspace()?;
    let dir = session_dir(&dot_browser, &name);
    std::fs::create_dir_all(&dir)?;
    set_permissions_0700(&dir)?;

    let session = session_file::SessionFile {
        version: 2,
        address: address.clone(),
        name: name.clone(),
        intro: intro.clone(),
        created_at: iso_now(),
        registered_by: None,
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    let path = dir.join("session.json");
    let content = serde_json::to_string_pretty(&session)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;

    Ok((address, name, token))
}

/// 用 token 验证浏览器扩展身份。
pub fn validate(storage: &Storage, token: &str) -> Result<AuthenticatedSession, IdentityError> {
    let (address, name) = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT address, name FROM browser_sessions WHERE token = ?1",
            [token],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| IdentityError::AgentNotRegistered)?
    };

    let mb = mailbox::get_by_address(storage, &address)?
        .ok_or_else(|| IdentityError::MailboxNotFound(address.clone()))?;

    Ok(AuthenticatedSession {
        address,
        name,
        workspace: mb.workspace,
        workspace_root: browser_workspace()?,
        pid: None,
    })
}

/// 注销浏览器扩展身份。
pub fn delete(storage: &Storage, token: &str) -> Result<(), IdentityError> {
    let (address, name) = {
        let conn = storage.conn();
        let row = conn
            .query_row(
                "SELECT address, name FROM browser_sessions WHERE token = ?1",
                [token],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|_| IdentityError::AgentNotRegistered)?;
        conn.execute("DELETE FROM browser_sessions WHERE token = ?1", [token])?;
        row
    };

    mailbox::mark_left(storage, &address)?;

    let dot_browser = browser_workspace()?;
    let dir = session_dir(&dot_browser, &name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }

    Ok(())
}

fn short_id() -> String {
    Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("")
        .to_string()
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("AGTALK_CONFIG_DIR");
            std::env::set_var("AGTALK_CONFIG_DIR", path);
            Self(previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref p) = self.0 {
                std::env::set_var("AGTALK_CONFIG_DIR", p);
            } else {
                std::env::remove_var("AGTALK_CONFIG_DIR");
            }
        }
    }

    #[test]
    fn browser_session_create_validate_delete() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        let storage = Storage::open_in_memory().unwrap();

        let (address, name, token) = create(
            &storage,
            Some("web-agent".to_string()),
            Some("browser".to_string()),
            Some("test".to_string()),
        )
        .unwrap();

        assert_eq!(name, "web-agent");

        let session = validate(&storage, &token).unwrap();
        assert_eq!(session.address, address);
        assert_eq!(session.name, "web-agent");
        assert_eq!(session.workspace, "test");

        delete(&storage, &token).unwrap();
        assert!(validate(&storage, &token).is_err());
    }
}
