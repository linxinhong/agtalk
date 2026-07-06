//! CLI daemon 管理：启动、停止、状态。

use crate::cli::output::{print_server_msg, CliError};
use crate::proto::ServerMsg;
use crate::server::daemon::{self, DaemonStatusFile};
use std::env;
use std::fs::OpenOptions;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DAEMON_CHILD_ENV: &str = "AGTALK_DAEMON_CHILD";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn start(json: bool) -> Result<(), String> {
    if daemon::is_running() {
        let msg = status_info().map_err(|e| e.message)?;
        print_server_msg(json, &msg);
        return Ok(());
    }

    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let log_path = daemon_log_path()?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("无法打开 daemon 日志文件 {}: {}", log_path.display(), e))?;

    let child = Command::new(exe)
        .arg("daemon")
        .arg("start")
        .env(DAEMON_CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        .spawn()
        .map_err(|e| e.to_string())?;

    let pid = child.id();

    // 轮询等待 daemon 写好状态文件并响应 HTTP
    let started = wait_for(|_| daemon::is_running(), START_TIMEOUT);
    if !started {
        return Err(format!(
            "daemon 未能及时启动 (pid {})，请检查日志 {}",
            pid,
            log_path.display()
        ));
    }

    let msg = status_info().map_err(|e| e.message)?;
    print_server_msg(json, &msg);
    Ok(())
}

/// 由子进程调用：真正启动 daemon 并阻塞，直到收到关闭信号。
pub fn run_server() -> Result<(), String> {
    let dot_agtalk = env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".agtalk");

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(daemon::start(dot_agtalk))
        .map_err(|e| e.to_string())
}

pub fn stop(json: bool) -> Result<(), String> {
    if !daemon::is_running() {
        let msg = status_info().map_err(|e| e.message)?;
        print_server_msg(json, &msg);
        return Err("daemon 未运行".to_string());
    }

    daemon::stop().map_err(|e| e.to_string())?;

    let stopped = wait_for(|running| !running, STOP_TIMEOUT);
    if !stopped {
        return Err("daemon 未能在超时内停止".to_string());
    }

    let msg = status_info().map_err(|e| e.message)?;
    print_server_msg(json, &msg);
    Ok(())
}

pub fn restart(json: bool) -> Result<(), String> {
    let _ = stop(json);
    start(json)
}

/// 返回当前 daemon 状态信息：先读状态文件，若运行中再调 HTTP 接口取实时指标。
pub fn status_info() -> Result<ServerMsg, CliError> {
    let default_config_path = crate::paths::config_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let default_db_path = crate::paths::db_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let file = match daemon::read_status_file().map_err(|e| CliError::from(e.to_string()))? {
        Some(f) => f,
        None => {
            return Ok(ServerMsg::DaemonStatus {
                pid: 0,
                start_time: 0,
                version: env!("CARGO_PKG_VERSION").to_string(),
                http_port: 0,
                uptime_seconds: 0,
                active_mailboxes: 0,
                pending_messages: 0,
                sse_subscribers: 0,
                config_path: default_config_path,
                db_path: default_db_path,
            })
        }
    };

    match fetch_live_status(&file) {
        Ok(msg) => Ok(msg),
        Err(_e) => {
            // HTTP 不可达时退化为状态文件基础信息
            Ok(file_to_status(file, 0))
        }
    }
}

pub fn is_running() -> bool {
    daemon::is_running()
}

/// 判断当前进程是否是被 `start()` spawn 出来的 daemon 子进程。
pub fn is_child_process() -> bool {
    env::var_os(DAEMON_CHILD_ENV).is_some()
}

fn wait_for<F>(mut predicate: F, timeout: Duration) -> bool
where
    F: FnMut(bool) -> bool,
{
    let start = Instant::now();
    loop {
        let running = daemon::is_running();
        if predicate(running) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn fetch_live_status(file: &DaemonStatusFile) -> Result<ServerMsg, String> {
    let url = format!("http://127.0.0.1:{}/api/v1/daemon/status", file.http_port);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP 状态码: {}", resp.status()));
    }
    resp.json::<ServerMsg>().map_err(|e| e.to_string())
}

fn file_to_status(file: DaemonStatusFile, uptime_seconds: u64) -> ServerMsg {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let uptime = if file.start_time > 0 {
        now.saturating_sub(file.start_time)
    } else {
        uptime_seconds
    };
    ServerMsg::DaemonStatus {
        pid: file.pid,
        start_time: file.start_time,
        version: file.version,
        http_port: file.http_port,
        uptime_seconds: uptime,
        active_mailboxes: 0,
        pending_messages: 0,
        sse_subscribers: 0,
        config_path: file.config_path,
        db_path: file.db_path,
    }
}

fn daemon_log_path() -> Result<std::path::PathBuf, String> {
    let dir = crate::paths::ensure_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("daemon.log"))
}
