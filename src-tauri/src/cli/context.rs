//! CLI 运行时上下文：当前进程对应的 session、daemon 地址等。

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
    /// 3. 已注册祖先 PID
    /// 4. 当前目录只有一个 session 时自动恢复
    /// 5. 多个 session 且无法判断 → identity_ambiguous
    pub fn current(as_name: Option<&str>) -> Result<Self, String> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".agtalk");

        let (pid, start_time, name) = if let Some(name) = as_name {
            Self::resolve_by_name(&dot_agtalk, name)?
        } else if let Some(name) = env::var_os("AGTALK_NAME") {
            let name = name
                .into_string()
                .map_err(|_| "AGTALK_NAME 不是有效 UTF-8".to_string())?;
            Self::resolve_by_name(&dot_agtalk, &name)?
        } else {
            Self::resolve_identity(&dot_agtalk)
                .or_else(|_| Self::auto_recover_single_session(&dot_agtalk))?
        };

        let session = session_file::read(&dot_agtalk, &name)
            .map_err(|e| format!("读取 session 失败: {}", e))?;

        let config = AgConfig::load().map_err(|e| e.to_string())?;
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

    /// 用于 `join`：不依赖 agents.json 中已注册的条目。
    /// 身份注册到**当前 shell 的 session leader**（沿父链上行、仍有有效 start_time
    /// 的最高存活祖先）。这样同一会话内后续命令（含子 shell）都能通过祖先链找到同一身份。
    ///
    /// 若找不到任何有效祖先（整条链都死掉），则回退到当前进程自身——
    /// 这比返回 start_time=0 的幽灵锚点更安全（后者会让 PID 复用防护失效）。
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
    ) -> Result<(u32, u64, String), String> {
        let session = session_file::read(dot_agtalk, name)
            .map_err(|_| format!("未找到身份 '{}' 的 session.json", name))?;

        let cur_pid = std::process::id();
        let mut sys = System::new_all();
        let cur_start = os_process_start_time(&mut sys, cur_pid);
        if cur_start == 0 {
            return Err("无法获取有效进程启动时间".into());
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

    /// 沿父链上行，找第一个在 agents.json 中注册**且仍然存活**的祖先。
    /// 跳过 start_time=0 或 OS 中已不存在的条目——它们可能是 PID 复用后的僵尸记录。
    fn resolve_identity(dot_agtalk: &Path) -> Result<(u32, u64, String), String> {
        let mut sys = System::new_all();
        sys.refresh_processes();

        let mut pid = std::process::id();

        // 先检查当前进程自身
        if let Some(entry) = agents_map::get_by_pid(dot_agtalk, pid).map_err(|e| e.to_string())? {
            let st = os_process_start_time(&mut sys, pid);
            if st != 0 && st == entry.start_time {
                return Ok((pid, st, entry.name));
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

            if let Some(entry) =
                agents_map::get_by_pid(dot_agtalk, ppid).map_err(|e| e.to_string())?
            {
                let st = os_process_start_time(&mut sys, ppid);
                // 只有 OS 实际启动时间与 agents.json 记录一致才认——防 PID 复用
                if st != 0 && st == entry.start_time {
                    return Ok((ppid, st, entry.name));
                }
                // 不匹配 → 可能是 PID 复用后的僵尸记录，继续上溯
            }

            pid = ppid;
        }

        Err("no_registered_ancestor".into())
    }

    /// 当前目录只有一个 session 时自动恢复：把当前 PID 注册到该 name。
    pub(crate) fn auto_recover_single_session(
        dot_agtalk: &Path,
    ) -> Result<(u32, u64, String), String> {
        let sessions = list_session_names(dot_agtalk)?;
        match sessions.len() {
            0 => Err(format!(
                "当前进程 (pid {}) 未在 agents.json 注册，且工作目录没有 session，先执行 agtalk join",
                std::process::id()
            )),
            1 => {
                let name = sessions.into_iter().next().unwrap();
                let cur_pid = std::process::id();
                let mut sys = System::new_all();
                let cur_start = os_process_start_time(&mut sys, cur_pid);
                if cur_start == 0 {
                    return Err("无法获取有效进程启动时间".into());
                }
                agents_map::register_pid(dot_agtalk, cur_pid, &name, cur_start)
                    .map_err(|e| e.to_string())?;
                let (anchor_pid, anchor_start) = session_anchor(cur_pid);
                if anchor_pid != cur_pid && anchor_start != 0 {
                    let _ = agents_map::register_pid(dot_agtalk, anchor_pid, &name, anchor_start);
                }
                Ok((cur_pid, cur_start, name))
            }
            _ => Err(format!(
                "identity_ambiguous: 工作目录有多个 session ({}), 请用 --as <name> 或 AGTALK_NAME 指定",
                sessions.join(", ")
            )),
        }
    }
}

/// 扫描 `.agtalk/*/` 下的所有 session.json，返回 name 列表。
fn list_session_names(dot_agtalk: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if !dot_agtalk.exists() {
        return Ok(names);
    }
    for entry in std::fs::read_dir(dot_agtalk).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
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
/// CLI 子进程生命周期极短，身份绑定到 session leader / shell 上，而不是每个 CLI 子进程。
///
/// macOS 上 sysinfo 对部分进程（如通过 launchd 启动的登录 shell）返回 start_time() == 0，
/// 因此只在拿到非零 start_time 时才推进 info，保留最后一个有效锚点。
/// 若整条链都无效（全死），回退到当前进程自身（start_time 可能 >0）。
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
    use crate::identity::session_file::{NotifyTarget, SessionFile};
    use tempfile::TempDir;

    #[test]
    fn list_session_names_finds_sessions() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "addr".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            command: "agtalk".to_string(),
            notify_channel: "none".to_string(),
            notify_target: NotifyTarget::None,
        };
        session_file::write(&dot, "nora", &session).unwrap();
        session_file::write(&dot, "quinn", &session).unwrap();
        let mut names = list_session_names(&dot).unwrap();
        names.sort();
        assert_eq!(names, vec!["nora", "quinn"]);
    }

    #[test]
    fn resolve_by_name_selects_session() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        session_file::write(&dot, "nora", &session).unwrap();

        let (_pid, _st, name) = Context::resolve_by_name(&dot, "nora").unwrap();
        assert_eq!(name, "nora");
        // 应自动注册当前 PID
        let entry = agents_map::get_by_pid(&dot, std::process::id())
            .unwrap()
            .unwrap();
        assert_eq!(entry.name, "nora");
    }

    #[test]
    fn auto_recover_single_session_works() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        session_file::write(&dot, "nora", &session).unwrap();

        let (_pid, _st, name) = Context::auto_recover_single_session(&dot).unwrap();
        assert_eq!(name, "nora");
        let entry = agents_map::get_by_pid(&dot, std::process::id())
            .unwrap()
            .unwrap();
        assert_eq!(entry.name, "nora");
    }

    #[test]
    fn auto_recover_ambiguous_fails() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        session_file::write(&dot, "nora", &session).unwrap();
        session_file::write(&dot, "quinn", &session).unwrap();

        let err = Context::auto_recover_single_session(&dot).unwrap_err();
        assert!(err.contains("identity_ambiguous"));
    }

    #[test]
    fn auto_recover_no_session_fails() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        std::fs::create_dir_all(&dot).unwrap();
        let err = Context::auto_recover_single_session(&dot).unwrap_err();
        assert!(err.contains("未在 agents.json 注册"));
    }
}
