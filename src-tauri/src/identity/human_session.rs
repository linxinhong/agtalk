//! system-human session：本机 human 客户端（popup/GUI）的认证会话。
//!
//! human 客户端不是 agent 进程，没有 PID + 文件系统认证锚，因此 daemon 在配置目录维护
//! `<config_dir>/human/session.json`（目录 0700、文件 0600），内含 human mailbox 的
//! address + 高熵 token。客户端凭 `X-AgTalk-Human-Token` 调用仅 human 可用的 API 并订阅
//! 统一 SSE。token 仅本机 human 客户端使用，绝不暴露给 agent（docs/design.md §2.3/§3.6）。

use super::IdentityError;
use crate::paths::{ensure_human_session_dir, set_permissions_0600};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanSession {
    pub version: u32,
    pub address: String,
    pub name: String,
    pub token: String,
    pub created_at: String,
}

fn session_path() -> Result<PathBuf, IdentityError> {
    Ok(ensure_human_session_dir()?.join("session.json"))
}

/// 确保 system-human session 存在：文件存在且 address 匹配时复用原 token；否则颁发新 token。
pub fn ensure(address: &str, name: &str) -> Result<HumanSession, IdentityError> {
    let path = session_path()?;
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(existing) = serde_json::from_str::<HumanSession>(&content) {
            if existing.address == address && !existing.token.is_empty() {
                return Ok(existing);
            }
        }
    }
    let session = HumanSession {
        version: SESSION_VERSION,
        address: address.to_string(),
        name: name.to_string(),
        token: Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let content = serde_json::to_string_pretty(&session)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;
    Ok(session)
}

/// 用 token 校验 human 客户端身份，返回其 session（含 address/name）。
pub fn validate(token: &str) -> Result<HumanSession, IdentityError> {
    if token.is_empty() {
        return Err(IdentityError::InvalidHumanToken);
    }
    let path = session_path()?;
    let content = std::fs::read_to_string(&path).map_err(|_| IdentityError::InvalidHumanToken)?;
    let session: HumanSession = serde_json::from_str(&content)?;
    if session.token != token {
        return Err(IdentityError::InvalidHumanToken);
    }
    Ok(session)
}

/// 读取本机 human session（客户端用）。文件缺失或非法返回 InvalidHumanToken，
/// 调用方应提示重启 daemon 重新颁发。
pub fn load() -> Result<HumanSession, IdentityError> {
    let path = session_path()?;
    let content = std::fs::read_to_string(&path).map_err(|_| IdentityError::InvalidHumanToken)?;
    let session: HumanSession = serde_json::from_str(&content)?;
    if session.token.is_empty() || session.address.is_empty() {
        return Err(IdentityError::InvalidHumanToken);
    }
    Ok(session)
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
    fn ensure_creates_session_0600_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());

        let s1 = ensure("addr-human", "human").unwrap();
        assert_eq!(s1.address, "addr-human");
        assert!(!s1.token.is_empty());

        let path = session_path().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "session.json 权限应为 0600");
        }

        // 再次 ensure 复用同一 token（幂等，不轮换）
        let s2 = ensure("addr-human", "human").unwrap();
        assert_eq!(s1.token, s2.token);
    }

    #[test]
    fn ensure_with_changed_address_reissues_token() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        let s1 = ensure("addr-1", "human").unwrap();
        let s2 = ensure("addr-2", "human").unwrap();
        assert_eq!(s2.address, "addr-2");
        assert_ne!(s1.token, s2.token, "address 变化应重新颁发 token");
    }

    #[test]
    fn validate_accepts_correct_token_rejects_wrong_or_empty() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        let s = ensure("addr-human", "human").unwrap();

        let ok = validate(&s.token).unwrap();
        assert_eq!(ok.address, "addr-human");

        assert!(validate("wrong-token").is_err());
        assert!(validate("").is_err());
    }
}
