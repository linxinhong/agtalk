//! CLI daemon 管理：启动、停止、状态。

use crate::server::daemon;
use std::env;
use std::process::{Command, Stdio};

const DAEMON_CHILD_ENV: &str = "AGTALK_DAEMON_CHILD";

pub fn start() -> Result<(), String> {
    let status = daemon::status();
    if status.starts_with("running") {
        return Err(format!("daemon 已在运行 ({status})"));
    }

    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let child = Command::new(exe)
        .arg("daemon")
        .arg("start")
        .env(DAEMON_CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;

    println!("daemon 启动中 (pid {})", child.id());
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

pub fn stop() -> Result<(), String> {
    daemon::stop().map_err(|e| e.to_string())
}

pub fn restart() -> Result<(), String> {
    let _ = daemon::stop();
    start()
}

pub fn status() -> String {
    daemon::status()
}

/// 判断当前进程是否是被 `start()` spawn 出来的 daemon 子进程。
pub fn is_child_process() -> bool {
    env::var_os(DAEMON_CHILD_ENV).is_some()
}
