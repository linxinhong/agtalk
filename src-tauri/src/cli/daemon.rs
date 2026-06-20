//! daemon 健康检查、stale 清理与 proxy 环境工具。

use crate::cli::client::Client;
use crate::ipc::ServerMsg;
use crate::paths;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// daemon 状态综合判断结果。
#[derive(Debug, Clone)]
pub enum DaemonState {
    /// 进程与 socket ping 均正常。
    Running { pid: u32 },
    /// PID 文件存在且进程存活，但 socket 尚未 ping 通（启动中或僵死）。
    Starting {
        pid: u32,
        reason: String,
    },
    /// PID/socket 文件存在但无法 ping 通；可能残留。
    Stale {
        pid: Option<u32>,
        socket_exists: bool,
        reason: String,
    },
    /// 未运行。
    NotRunning,
}

impl std::fmt::Display for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonState::Running { pid } => write!(f, "运行中 (pid {})", pid),
            DaemonState::Starting { pid, .. } => write!(f, "启动中 (pid {})", pid),
            DaemonState::Stale { pid, socket_exists, reason } => {
                write!(f, "检测到残留状态：{}", reason)?;
                if let Some(pid) = pid {
                    write!(f, "，pid={}", pid)?;
                }
                write!(f, "，socket 文件存在={}", socket_exists)
            }
            DaemonState::NotRunning => write!(f, "未运行"),
        }
    }
}

pub fn socket_path() -> String {
    paths::socket_path()
}

pub fn pid_path() -> std::path::PathBuf {
    paths::pid_path()
}

/// 尝试读取并解析 pid 文件。
pub fn read_pid() -> Option<u32> {
    std::fs::read_to_string(pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// 通过 kill -0 检查进程是否存活。
pub fn is_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 对 socket 发起 Ping/Pong 检查。
pub async fn ping_socket(socket_path: &str) -> Result<()> {
    let mut client = Client::connect(socket_path).await?;
    match client.ping().await? {
        ServerMsg::Ok { data } => {
            if data.get("pong").and_then(|v| v.as_bool()).unwrap_or(false) {
                Ok(())
            } else {
                anyhow::bail!("ping 返回数据异常: {:?}", data)
            }
        }
        other => anyhow::bail!("ping 返回异常: {:?}", other),
    }
}

/// 综合判断 daemon 当前状态。
pub async fn check_state() -> DaemonState {
    let socket_path = paths::socket_path();
    let pid = read_pid();
    let pid_alive = pid.map(is_process_alive).unwrap_or(false);
    let socket_exists = Path::new(&socket_path).exists();

    // 优先尝试 ping；ping 通过即认为运行中。
    if socket_exists && ping_socket(&socket_path).await.is_ok() {
        return DaemonState::Running {
            pid: pid.unwrap_or(0),
        };
    }

    // ping 失败但 pid 存活，可能是启动中或僵死。
    if let Some(pid) = pid {
        if pid_alive {
            return DaemonState::Starting {
                pid,
                reason: format!(
                    "pid {} 存活，但 socket {} 无法 ping",
                    pid, socket_path
                ),
            };
        }
        // pid 不存在但文件残留。
        return DaemonState::Stale {
            pid: Some(pid),
            socket_exists,
            reason: format!(
                "pid 文件指向 {}，但进程已不存在",
                pid
            ),
        };
    }

    // 无 pid 文件，但 socket 文件残留。
    if socket_exists {
        return DaemonState::Stale {
            pid: None,
            socket_exists,
            reason: format!("无 pid 文件，但 socket 文件 {} 残留", socket_path),
        };
    }

    DaemonState::NotRunning
}

/// 清理 stale 的 pid/socket 文件。
pub fn clean_stale_files() -> Result<()> {
    let pid_path = paths::pid_path();
    let socket_path = paths::socket_path();
    if pid_path.exists() {
        std::fs::remove_file(&pid_path)
            .with_context(|| format!("无法删除残留 pid 文件: {:?}", pid_path))?;
    }
    if Path::new(&socket_path).exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("无法删除残留 socket 文件: {}", socket_path))?;
    }
    Ok(())
}

/// 等待 daemon 进入 Running 状态，超时返回错误。
pub async fn wait_for_running(max_wait: Duration) -> Result<DaemonState> {
    let interval = Duration::from_millis(100);
    let start = std::time::Instant::now();
    loop {
        let state = check_state().await;
        if matches!(state, DaemonState::Running { .. }) {
            return Ok(state);
        }
        if start.elapsed() >= max_wait {
            anyhow::bail!(
                "等待 daemon 就绪超时（{}ms）",
                max_wait.as_millis()
            );
        }
        tokio::time::sleep(interval).await;
    }
}

/// 返回与代理相关的环境变量摘要，用于诊断输出。
pub fn proxy_env_summary() -> String {
    let mut parts = Vec::new();
    for name in &["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
        if let Ok(val) = std::env::var(name) {
            parts.push(format!("{}={}", name, val));
        }
    }
    if parts.is_empty() {
        return "无网络代理环境变量".to_string();
    }
    parts.join("; ")
}

/// 检测是否设置了可能干扰本地 Unix socket 的网络代理变量。
pub fn has_network_proxy() -> bool {
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"]
        .iter()
        .any(|n| std::env::var(n).is_ok())
}

/// 构造包含 localhost/127.0.0.1/::1 的 NO_PROXY 值，合并并去重用户现有设置。
pub fn no_proxy_value() -> String {
    let mut set = HashSet::new();
    for var in ["NO_PROXY", "no_proxy"] {
        if let Ok(val) = std::env::var(var) {
            for item in val.split(',') {
                let item = item.trim();
                if !item.is_empty() {
                    set.insert(item.to_string());
                }
            }
        }
    }
    for item in ["localhost", "127.0.0.1", "::1"] {
        set.insert(item.to_string());
    }
    let mut items: Vec<String> = set.into_iter().collect();
    items.sort();
    items.join(",")
}

/// 为启动 daemon 的子进程设置 NO_PROXY（保留用户其他代理变量）。
pub fn apply_no_proxy(cmd: &mut Command) {
    cmd.env("NO_PROXY", no_proxy_value());
    // 某些工具会读取小写形式，同时设置增加兼容性。
    cmd.env("no_proxy", no_proxy_value());
}

/// 生成连接失败时的增强诊断文本。
pub fn connection_diagnostic(socket_path: &str, err: &str) -> String {
    let mut lines = vec![
        format!("无法连接 daemon: {}", err),
        format!("socket 路径: {}", socket_path),
    ];
    if has_network_proxy() {
        lines.push(format!(
            "检测到网络代理环境变量：{}。agtalk 本地 IPC 使用 Unix domain socket，不应经过网络代理；请检查代理工具是否拦截本地连接，或是否隔离了 HOME / XDG / AGTALK_CONFIG_DIR 环境变量。",
            proxy_env_summary()
        ));
    }
    lines.push("可尝试执行 `agtalk daemon restart` 修复 stale 的 pid/socket 文件。".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_merges_localhost() {
        // 测试不依赖环境；no_proxy_value 会读取当前 env，至少保证 localhost 存在。
        let val = no_proxy_value();
        assert!(val.contains("localhost"), "{}", val);
        assert!(val.contains("127.0.0.1"), "{}", val);
        assert!(val.contains("::1"), "{}", val);
    }

    #[test]
    fn no_proxy_deduplicates_and_sorts() {
        // 临时设置环境变量并验证去重。
        let old = std::env::var("NO_PROXY").ok();
        std::env::set_var("NO_PROXY", "example.com,localhost,example.com");
        let val = no_proxy_value();
        assert_eq!(val.split(',').filter(|s| *s == "example.com").count(), 1);
        // 重置
        match old {
            Some(v) => std::env::set_var("NO_PROXY", v),
            None => std::env::remove_var("NO_PROXY"),
        }
    }

    #[test]
    fn proxy_env_summary_handles_proxy_vars() {
        let summary = proxy_env_summary();
        if has_network_proxy() {
            assert!(
                summary.contains("HTTP_PROXY")
                    || summary.contains("HTTPS_PROXY")
                    || summary.contains("ALL_PROXY")
                    || summary.contains("http_proxy")
                    || summary.contains("https_proxy")
                    || summary.contains("all_proxy"),
                "{}",
                summary
            );
        } else {
            assert_eq!(summary, "无网络代理环境变量");
        }
    }
}
