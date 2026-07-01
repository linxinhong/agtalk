//! CLI 运行时上下文：当前进程对应的 session、daemon 地址等。

use crate::config::AgConfig;
use crate::identity::agents_map::{self, AgentEntry};
use crate::identity::session_file;
use std::env;
use std::path::{Path, PathBuf};
use sysinfo::{Pid, System};

pub struct Context {
    pub dot_agtalk: PathBuf,
    pub address: String,
    pub name: String,
    pub pid: u32,
    pub start_time: u64,
    pub base_url: String,
}

impl Context {
    pub fn current() -> Result<Self, String> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".agtalk");

        let (pid, start_time, entry) =
            Self::resolve_identity(&dot_agtalk).map_err(|e| e.to_string())?;

        let session = session_file::read(&dot_agtalk, &entry.name)
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
    /// CLI 子进程生命周期极短，因此把身份注册到**会话根进程**（shell/session leader）上，
    /// 后续同一会话内的命令（包括命令替换产生的子 shell）都能通过祖先链找到同一身份。
    pub fn pre_join() -> Result<Self, String> {
        let dot_agtalk = env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".agtalk");

        let (pid, start_time) = session_anchor(std::process::id());

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

    fn resolve_identity(dot_agtalk: &Path) -> Result<(u32, u64, AgentEntry), String> {
        let mut sys = System::new_all();
        sys.refresh_processes();

        let mut pid = std::process::id();
        let start_time = current_process_start_time(pid)?;

        if let Some(entry) = agents_map::get_by_pid(dot_agtalk, pid).map_err(|e| e.to_string())? {
            return Ok((pid, start_time, entry));
        }

        while let Some(process) = sys.process(Pid::from(pid as usize)) {
            let parent = match process.parent() {
                Some(p) => p,
                None => break,
            };

            let ppid = parent.as_u32();
            if ppid == 0 || ppid == pid {
                break;
            }

            let parent_proc = match sys.process(parent) {
                Some(p) => p,
                None => break,
            };

            if let Some(entry) =
                agents_map::get_by_pid(dot_agtalk, ppid).map_err(|e| e.to_string())?
            {
                return Ok((ppid, parent_proc.start_time(), entry));
            }

            pid = ppid;
        }

        Err(format!(
            "当前进程 (pid {}) 及其所有已注册祖先都未在 agents.json 中，先执行 agtalk join",
            std::process::id()
        ))
    }
}

fn current_process_start_time(pid: u32) -> Result<u64, String> {
    let mut sys = System::new_all();
    sys.refresh_processes();
    let process = sys
        .process(Pid::from(pid as usize))
        .ok_or("无法获取当前进程信息")?;
    Ok(process.start_time())
}

/// 返回进程所在会话的稳定锚点（最顶层非 root 祖先）。
/// CLI 命令极短，身份绑定到 session leader / shell 上，而不是每个 CLI 子进程。
fn session_anchor(pid: u32) -> (u32, u64) {
    let mut sys = System::new_all();
    sys.refresh_processes();

    let mut cur = pid;
    let mut info = sys
        .process(Pid::from(cur as usize))
        .map(|p| (cur, p.start_time()))
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
        info = (ppid, parent_proc.start_time());
        cur = ppid;
    }

    info
}
