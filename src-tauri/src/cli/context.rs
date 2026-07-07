//! CLI 运行时上下文：当前进程对应的 session、daemon 地址等。

use crate::cli::context_error::IdentityResolutionError;
use crate::config::AgConfig;
use crate::identity::agents_map;
use crate::identity::session_file;
use std::env;
use std::path::{Path, PathBuf};
use sysinfo::{Pid, System};

#[derive(Clone)]
pub struct Context {
    pub dot_agtalk: PathBuf,
    pub address: String,
    pub name: String,
    pub pid: u32,
    pub start_time: u64,
    pub base_url: String,
}

impl Context {
    /// 解析当前命令使用的身份。
    ///
    /// 选择优先级：
    /// 1. `--as <name>`
    /// 2. `AGTALK_NAME=<name>`
    /// 3. 已注册祖先 PID（自动清理 stale anchor）
    /// 4. 当前目录只有一个 session 时自动恢复
    /// 5. 多个 session 且无法判断 → identity_ambiguous
    pub fn current(as_name: Option<&str>) -> Result<Self, IdentityResolutionError> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| IdentityResolutionError::Io(e.to_string()))?
            .join(".agtalk");

        let (pid, start_time, name) = if let Some(name) = as_name {
            Self::resolve_by_name(&dot_agtalk, name)?
        } else if let Some(name) = env::var_os("AGTALK_NAME") {
            let name = name
                .into_string()
                .map_err(|_| IdentityResolutionError::Io("AGTALK_NAME 不是有效 UTF-8".into()))?;
            Self::resolve_by_name(&dot_agtalk, &name)?
        } else {
            Self::resolve_identity(&dot_agtalk)
                .or_else(|_| Self::auto_recover_single_session(&dot_agtalk))?
        };

        let session = session_file::read(&dot_agtalk, &name).map_err(|e| match e {
            crate::identity::IdentityError::Io(_) if !session_path_exists(&dot_agtalk, &name) => {
                IdentityResolutionError::SessionMissing { name: name.clone() }
            }
            _ => IdentityResolutionError::InvalidSession {
                name: name.clone(),
                reason: e.to_string(),
            },
        })?;

        let config = AgConfig::load().map_err(|e| IdentityResolutionError::Io(e.to_string()))?;
        let base_url = format!("http://127.0.0.1:{}", config.http_port);

        Ok(Self {
            dot_agtalk,
            address: session.address,
            name: session.name,
            pid,
            start_time,
            base_url,
        })
    }

    /// 构造一个不需要本地身份、只用于访问 daemon 的上下文。
    /// 用于 `id lookup` 等“无身份命令”。
    pub fn daemon_only() -> Result<Self, IdentityResolutionError> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| IdentityResolutionError::Io(e.to_string()))?
            .join(".agtalk");
        let config = AgConfig::load().map_err(|e| IdentityResolutionError::Io(e.to_string()))?;
        let base_url = format!("http://127.0.0.1:{}", config.http_port);
        Ok(Self {
            dot_agtalk,
            address: String::new(),
            name: String::new(),
            pid: 0,
            start_time: 0,
            base_url,
        })
    }

    /// 用于 `join`：不依赖 agents.json 中已注册的条目。
    pub fn pre_join() -> Result<Self, String> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".agtalk");

        let (pid, start_time) = session_anchor(std::process::id());
        if start_time == 0 {
            return Err("无法获取有效进程启动时间（session leader 可能已退出），请重试".into());
        }

        let config = AgConfig::load().map_err(|e| e.to_string())?;
        let base_url = format!("http://127.0.0.1:{}", config.http_port);

        Ok(Self {
            dot_agtalk,
            address: String::new(),
            name: String::new(),
            pid,
            start_time,
            base_url,
        })
    }

    pub(crate) fn resolve_by_name(
        dot_agtalk: &Path,
        name: &str,
    ) -> Result<(u32, u64, String), IdentityResolutionError> {
        if !session_path_exists(dot_agtalk, name) {
            return Err(IdentityResolutionError::SessionMissing {
                name: name.to_string(),
            });
        }

        let session = session_file::read(dot_agtalk, name).map_err(|e| {
            IdentityResolutionError::InvalidSession {
                name: name.to_string(),
                reason: e.to_string(),
            }
        })?;

        let cur_pid = std::process::id();
        let mut sys = System::new_all();
        let cur_start = os_process_start_time(&mut sys, cur_pid);
        if cur_start == 0 {
            return Err(IdentityResolutionError::Io(
                "无法获取有效进程启动时间".into(),
            ));
        }

        // 注册当前进程，便于当前命令自身被识别
        let _ = agents_map::register_pid(dot_agtalk, cur_pid, &session.name, cur_start);
        // 同时注册 session anchor，便于同一会话内的子/孙进程通过父链找到身份
        let (anchor_pid, anchor_start) = session_anchor(cur_pid);
        if anchor_pid != cur_pid && anchor_start != 0 {
            let _ = agents_map::register_pid(dot_agtalk, anchor_pid, &session.name, anchor_start);
        }

        Ok((cur_pid, cur_start, session.name))
    }

    /// 沿父链上行，找第一个在 agents.json 中注册**且 session.json 仍然有效**的祖先。
    /// 跳过 start_time=0、OS 中已不存在、或 session 缺失/损坏的条目。
    fn resolve_identity(dot_agtalk: &Path) -> Result<(u32, u64, String), IdentityResolutionError> {
        let mut sys = System::new_all();
        sys.refresh_processes();

        let mut pid = std::process::id();

        // 先检查当前进程自身
        if let Some(entry) = agents_map::get_by_pid(dot_agtalk, pid).map_err(io_err)? {
            if let Some(result) = Self::validate_anchor(dot_agtalk, pid, &entry, &mut sys)? {
                return Ok(result);
            }
        }

        // 沿父链上行
        while let Some(process) = sys.process(Pid::from(pid as usize)) {
            let parent = match process.parent() {
                Some(p) => p,
                None => break,
            };

            let ppid = parent.as_u32();
            if ppid == 0 || ppid == pid {
                break;
            }

            if let Some(entry) = agents_map::get_by_pid(dot_agtalk, ppid).map_err(io_err)? {
                if let Some(result) = Self::validate_anchor(dot_agtalk, ppid, &entry, &mut sys)? {
                    return Ok(result);
                }
            }

            pid = ppid;
        }

        // 父链耗尽，让 auto_recover_single_session 尝试从 session 目录恢复。
        Err(IdentityResolutionError::Required)
    }

    /// 校验 agents.json 中的一个 pid entry：
    ///   - OS 进程存在且 start_time 匹配（防 PID 复用）
    ///   - 对应 session.json 存在且可读
    ///   - 无效时删除该 pid entry，返回 None 让调用方继续查找。
    pub(crate) fn validate_anchor(
        dot_agtalk: &Path,
        pid: u32,
        entry: &agents_map::AgentEntry,
        sys: &mut System,
    ) -> Result<Option<(u32, u64, String)>, IdentityResolutionError> {
        let st = os_process_start_time(sys, pid);
        if st == 0 || st != entry.start_time {
            // PID 复用或进程已死：清理 stale anchor
            let _ = agents_map::remove_pid(dot_agtalk, pid);
            return Ok(None);
        }

        match session_file::read(dot_agtalk, &entry.name) {
            Ok(session) if session.name == entry.name => Ok(Some((pid, st, entry.name.clone()))),
            Ok(_) => {
                // session 存在但 name 不匹配，清理并继续
                let _ = agents_map::remove_pid(dot_agtalk, pid);
                Ok(None)
            }
            Err(crate::identity::IdentityError::Io(_))
                if !session_path_exists(dot_agtalk, &entry.name) =>
            {
                // session 文件缺失：清理 stale anchor
                let _ = agents_map::remove_pid(dot_agtalk, pid);
                Ok(None)
            }
            Err(e) => {
                // session 损坏：清理锚点并返回明确错误
                let _ = agents_map::remove_pid(dot_agtalk, pid);
                Err(IdentityResolutionError::InvalidSession {
                    name: entry.name.clone(),
                    reason: e.to_string(),
                })
            }
        }
    }

    /// 当前目录只有一个 session 时自动恢复：把当前 PID 注册到该 name。
    pub(crate) fn auto_recover_single_session(
        dot_agtalk: &Path,
    ) -> Result<(u32, u64, String), IdentityResolutionError> {
        let sessions = list_session_names(dot_agtalk)?;
        match sessions.len() {
            0 => Err(IdentityResolutionError::Required),
            1 => {
                let name = sessions.into_iter().next().unwrap();
                let cur_pid = std::process::id();
                let mut sys = System::new_all();
                let cur_start = os_process_start_time(&mut sys, cur_pid);
                if cur_start == 0 {
                    return Err(IdentityResolutionError::Io(
                        "无法获取有效进程启动时间".into(),
                    ));
                }
                agents_map::register_pid(dot_agtalk, cur_pid, &name, cur_start)
                    .map_err(|e| IdentityResolutionError::Io(e.to_string()))?;
                let (anchor_pid, anchor_start) = session_anchor(cur_pid);
                if anchor_pid != cur_pid && anchor_start != 0 {
                    let _ = agents_map::register_pid(dot_agtalk, anchor_pid, &name, anchor_start);
                }
                Ok((cur_pid, cur_start, name))
            }
            _ => Err(IdentityResolutionError::Ambiguous(sessions)),
        }
    }
}

fn session_path_exists(dot_agtalk: &Path, name: &str) -> bool {
    dot_agtalk.join(name).join("session.json").exists()
}

fn io_err(e: crate::identity::IdentityError) -> IdentityResolutionError {
    IdentityResolutionError::Io(e.to_string())
}

/// 扫描 `.agtalk/*/` 下的所有 session.json，返回 name 列表。
fn list_session_names(dot_agtalk: &Path) -> Result<Vec<String>, IdentityResolutionError> {
    let mut names = Vec::new();
    if !dot_agtalk.exists() {
        return Ok(names);
    }
    for entry in
        std::fs::read_dir(dot_agtalk).map_err(|e| IdentityResolutionError::Io(e.to_string()))?
    {
        let entry = entry.map_err(|e| IdentityResolutionError::Io(e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| IdentityResolutionError::Io(e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let session_path = entry.path().join("session.json");
        if session_path.exists() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// 查 OS 中某 PID 的启动时间；进程不存在或读不到返回 0。
fn os_process_start_time(sys: &mut System, pid: u32) -> u64 {
    sys.refresh_processes();
    sys.process(Pid::from(pid as usize))
        .map(|p| p.start_time())
        .unwrap_or(0)
}

/// 返回进程所在会话的稳定锚点：沿父链上行，取**最高且存活、start_time>0**的祖先。
fn session_anchor(pid: u32) -> (u32, u64) {
    let mut sys = System::new_all();
    sys.refresh_processes();

    let mut cur = pid;
    let mut info = sys
        .process(Pid::from(cur as usize))
        .map(|p| (cur, p.start_time()))
        .filter(|(_, st)| *st > 0)
        .unwrap_or((cur, 0));

    while let Some(process) = sys.process(Pid::from(cur as usize)) {
        let parent = match process.parent() {
            Some(p) => p,
            None => break,
        };
        let ppid = parent.as_u32();
        if ppid == 0 || ppid == cur {
            break;
        }
        let parent_proc = match sys.process(parent) {
            Some(p) => p,
            None => break,
        };
        let parent_start = parent_proc.start_time();
        if parent_start > 0 {
            info = (ppid, parent_start);
        }
        cur = ppid;
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::agents_map;
    use crate::identity::session_file::{NotifyTarget, SessionFile};
    use tempfile::TempDir;

    fn write_session(dot: &Path, name: &str) {
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: name.to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            command: "agtalk".to_string(),
            notify_channel: "none".to_string(),
            notify_target: NotifyTarget::None,
        };
        session_file::write(dot, name, &session).unwrap();
    }

    fn current_pid_start_time() -> (u32, u64) {
        let pid = std::process::id();
        let mut sys = System::new_all();
        sys.refresh_processes();
        let start = sys
            .process(Pid::from(pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(0);
        (pid, start)
    }

    #[test]
    fn list_session_names_finds_sessions() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");
        write_session(&dot, "quinn");
        let mut names = list_session_names(&dot).unwrap();
        names.sort();
        assert_eq!(names, vec!["nora", "quinn"]);
    }

    #[test]
    fn resolve_by_name_selects_session() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");

        let (_pid, _st, name) = Context::resolve_by_name(&dot, "nora").unwrap();
        assert_eq!(name, "nora");
        let entry = agents_map::get_by_pid(&dot, std::process::id())
            .unwrap()
            .unwrap();
        assert_eq!(entry.name, "nora");
    }

    #[test]
    fn resolve_by_name_missing_session() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");

        let err = Context::resolve_by_name(&dot, "nora").unwrap_err();
        assert_eq!(err.code(), "identity_session_missing");
    }

    #[test]
    fn resolve_by_name_invalid_session() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(dot.join("nora")).unwrap();
        std::fs::write(dot.join("nora").join("session.json"), "not json").unwrap();

        let err = Context::resolve_by_name(&dot, "nora").unwrap_err();
        assert_eq!(err.code(), "identity_invalid_session");
    }

    #[test]
    fn auto_recover_single_session_works() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");

        let (_pid, _st, name) = Context::auto_recover_single_session(&dot).unwrap();
        assert_eq!(name, "nora");
        let entry = agents_map::get_by_pid(&dot, std::process::id())
            .unwrap()
            .unwrap();
        assert_eq!(entry.name, "nora");
    }

    #[test]
    fn auto_recover_ambiguous_returns_candidates() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");
        write_session(&dot, "quinn");

        let err = Context::auto_recover_single_session(&dot).unwrap_err();
        assert_eq!(err.code(), "identity_ambiguous");
        match err {
            IdentityResolutionError::Ambiguous(names) => {
                let mut names = names;
                names.sort();
                assert_eq!(names, vec!["nora", "quinn"]);
            }
            _ => panic!("expected Ambiguous"),
        }
    }

    #[test]
    fn auto_recover_no_session_returns_required() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot).unwrap();
        let err = Context::auto_recover_single_session(&dot).unwrap_err();
        assert_eq!(err.code(), "identity_required");
    }

    #[test]
    fn resolve_identity_uses_current_pid_anchor() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");

        let (cur_pid, cur_start) = current_pid_start_time();
        agents_map::register_pid(&dot, cur_pid, "nora", cur_start).unwrap();

        let (pid, _st, name) = Context::resolve_identity(&dot).unwrap();
        assert_eq!(name, "nora");
        assert_eq!(pid, cur_pid);
    }

    #[test]
    fn validate_anchor_removes_stale_pid() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        write_session(&dot, "nora");

        // 找一个当前 OS 中确实不存在的 pid。
        let mut sys = System::new_all();
        sys.refresh_processes();
        let stale_pid = (1..=u32::MAX)
            .rev()
            .find(|pid| sys.process(Pid::from(*pid as usize)).is_none())
            .expect("should find a non-existent pid");

        agents_map::register_pid(&dot, stale_pid, "nora", 1_700_000_000).unwrap();

        let entry = agents_map::AgentEntry {
            name: "nora".into(),
            start_time: 1_700_000_000,
        };
        let result = Context::validate_anchor(&dot, stale_pid, &entry, &mut sys).unwrap();
        assert!(result.is_none());
        assert!(agents_map::get_by_pid(&dot, stale_pid).unwrap().is_none());
    }

    #[test]
    fn validate_anchor_removes_anchor_when_session_missing() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");

        let (cur_pid, cur_start) = current_pid_start_time();
        agents_map::register_pid(&dot, cur_pid, "ghost", cur_start).unwrap();

        let entry = agents_map::AgentEntry {
            name: "ghost".into(),
            start_time: cur_start,
        };
        let mut sys = System::new_all();
        sys.refresh_processes();
        let result = Context::validate_anchor(&dot, cur_pid, &entry, &mut sys).unwrap();
        assert!(result.is_none());
        assert!(agents_map::get_by_pid(&dot, cur_pid).unwrap().is_none());
    }

    #[test]
    fn resolve_identity_removes_anchor_when_session_missing() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");

        // 注册当前进程到一个不存在的 session
        let (cur_pid, cur_start) = current_pid_start_time();
        agents_map::register_pid(&dot, cur_pid, "ghost", cur_start).unwrap();

        let err = Context::resolve_identity(&dot);
        // resolve_identity 失败后会进入 auto_recover，而目录没有 session，最终 Required
        assert_eq!(err.unwrap_err().code(), "identity_required");
        // stale anchor 应被清理
        assert!(agents_map::get_by_pid(&dot, cur_pid).unwrap().is_none());
    }
}
