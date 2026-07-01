//! 认证链：PID → agents.json → name → session.json → UUID。

use super::agents_map;
use super::session_file;
use super::IdentityError;
use crate::storage::Storage;
use std::path::Path;
use sysinfo::{Pid, System};

#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    pub address: String,
    pub name: String,
    pub workspace: String,
    pub pid: Option<u32>,
}

/// 认证一个请求持有的 address。
///
/// 流程：
/// 1. 从 DB 查 address 对应 mailbox，取得 name/workspace。
/// 2. 读取 `<dot_agtalk>/<name>/session.json`，校验 address 一致。
/// 3. 若提供 pid + start_time，校验 OS 中该 pid 的启动时间一致，且 agents.json 中记录匹配。
pub fn authenticate(
    storage: &Storage,
    dot_agtalk: &Path,
    address: &str,
    pid: Option<u32>,
    start_time: Option<u64>,
) -> Result<AuthenticatedSession, IdentityError> {
    let mb = storage
        .conn()
        .query_row(
            "SELECT name, workspace FROM mailboxes WHERE address = ?1",
            [address],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| IdentityError::MailboxNotFound(address.to_string()))?;

    let (name, workspace) = mb;

    let session = session_file::read(dot_agtalk, &name)?;
    if session.address != address {
        return Err(IdentityError::SessionMismatch);
    }

    if let (Some(pid), Some(start_time)) = (pid, start_time) {
        validate_pid(dot_agtalk, pid, start_time, &name)?;
    }

    Ok(AuthenticatedSession {
        address: address.to_string(),
        name,
        workspace,
        pid,
    })
}

fn validate_pid(
    dot_agtalk: &Path,
    pid: u32,
    start_time: u64,
    name: &str,
) -> Result<(), IdentityError> {
    let entry = agents_map::get_by_pid(dot_agtalk, pid)?;
    let entry = entry.ok_or(IdentityError::AgentNotRegistered)?;
    if entry.name != name || entry.start_time != start_time {
        return Err(IdentityError::AgentNotRegistered);
    }

    let mut sys = System::new_all();
    sys.refresh_processes();
    let process = sys
        .process(Pid::from(pid as usize))
        .ok_or(IdentityError::ProcessNotFound)?;
    if process.start_time() != start_time {
        return Err(IdentityError::PidReused);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::mailbox;
    use crate::identity::session_file::SessionFile;
    use crate::storage::Storage;
    use tempfile::TempDir;

    #[test]
    fn auth_success_without_pid() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let addr = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
        let session = SessionFile {
            address: addr.clone(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        };
        session_file::write(&dot, "nora", &session).unwrap();

        let s = authenticate(&storage, &dot, &addr, None, None).unwrap();
        assert_eq!(s.name, "nora");
    }

    #[test]
    fn auth_fails_when_session_address_mismatch() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let addr = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
        let session = SessionFile {
            address: "00000000-0000-0000-0000-000000000000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        };
        session_file::write(&dot, "nora", &session).unwrap();

        assert!(authenticate(&storage, &dot, &addr, None, None).is_err());
    }
}
