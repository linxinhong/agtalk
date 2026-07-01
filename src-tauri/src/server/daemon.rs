//! daemon 生命周期：启动、停止、状态、单例锁。

use crate::config::AgConfig;
use crate::identity::mailbox;
use crate::paths::{daemon_pid_path, set_permissions_0600};
use crate::server::http::routes;
use crate::server::state::AppState;
use crate::storage::Storage;
use std::net::SocketAddr;
use std::path::PathBuf;
use sysinfo::{Pid, System};
use tokio::signal;
use tracing::{error, info};

const SHUTDOWN_TIMEOUT_SECONDS: u64 = 5;

#[derive(Debug)]
pub enum DaemonError {
    AlreadyRunning,
    NotRunning,
    Io(std::io::Error),
    Storage(crate::storage::StorageError),
    Config(crate::config::ConfigError),
    Bind(std::io::Error),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::AlreadyRunning => write!(f, "daemon 已经在运行"),
            DaemonError::NotRunning => write!(f, "daemon 未运行"),
            DaemonError::Io(e) => write!(f, "IO 错误: {}", e),
            DaemonError::Storage(e) => write!(f, "存储错误: {}", e),
            DaemonError::Config(e) => write!(f, "配置错误: {}", e),
            DaemonError::Bind(e) => write!(f, "绑定端口失败: {}", e),
        }
    }
}

impl From<std::io::Error> for DaemonError {
    fn from(e: std::io::Error) -> Self {
        DaemonError::Io(e)
    }
}

impl From<crate::paths::PathsError> for DaemonError {
    fn from(e: crate::paths::PathsError) -> Self {
        DaemonError::Io(std::io::Error::other(e.to_string()))
    }
}

impl From<crate::storage::StorageError> for DaemonError {
    fn from(e: crate::storage::StorageError) -> Self {
        DaemonError::Storage(e)
    }
}

impl From<crate::config::ConfigError> for DaemonError {
    fn from(e: crate::config::ConfigError) -> Self {
        DaemonError::Config(e)
    }
}

pub async fn start(dot_agtalk: PathBuf) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        use nix::unistd::setsid;
        let _ = setsid();
    }

    let _ = tracing_subscriber::fmt::try_init();

    if is_running()? {
        return Err(DaemonError::AlreadyRunning);
    }

    let config = AgConfig::load()?;
    let storage = Storage::open()?;

    // 确保 human mailbox 存在
    if let Err(e) = mailbox::ensure_human(&storage, &config.human) {
        error!("创建 human mailbox 失败: {}", e);
    }

    write_pid_file()?;

    let state = AppState::new(storage, config.clone(), dot_agtalk);
    let app = routes(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(DaemonError::Bind)?;
    info!("daemon 监听 127.0.0.1:{}", config.http_port);

    let server = axum::serve(listener, app);
    let shutdown = server.with_graceful_shutdown(shutdown_signal());

    if let Err(e) = shutdown.await {
        error!("server 异常退出: {}", e);
    }

    remove_pid_file();
    info!("daemon 已停止");
    Ok(())
}

pub fn stop() -> Result<(), DaemonError> {
    let pid = read_pid_file()?;
    if pid == 0 {
        return Err(DaemonError::NotRunning);
    }

    let mut sys = System::new_all();
    sys.refresh_processes();
    if sys.process(Pid::from(pid as usize)).is_some() {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid as NixPid;
            let _ = kill(NixPid::from_raw(pid as i32), Signal::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            return Err(DaemonError::NotRunning);
        }
    } else {
        remove_pid_file();
        return Err(DaemonError::NotRunning);
    }

    Ok(())
}

pub fn status() -> String {
    match is_running() {
        Ok(true) => {
            if let Ok(pid) = read_pid_file() {
                format!("running (pid {})", pid)
            } else {
                "running".to_string()
            }
        }
        _ => "stopped".to_string(),
    }
}

fn is_running() -> Result<bool, DaemonError> {
    let pid = read_pid_file()?;
    if pid == 0 {
        return Ok(false);
    }
    let mut sys = System::new_all();
    sys.refresh_processes();
    Ok(sys.process(Pid::from(pid as usize)).is_some())
}

fn read_pid_file() -> Result<u32, DaemonError> {
    let path = daemon_pid_path()?;
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(content.trim().parse::<u32>().unwrap_or(0))
}

fn write_pid_file() -> Result<(), DaemonError> {
    let path = daemon_pid_path()?;
    let pid = std::process::id().to_string();
    std::fs::write(&path, pid)?;
    set_permissions_0600(&path)?;
    Ok(())
}

fn remove_pid_file() {
    if let Ok(path) = daemon_pid_path() {
        let _ = std::fs::remove_file(path);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("注册 SIGTERM 失败");
        sig.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("收到 SIGINT，开始关闭..."),
        _ = terminate => info!("收到 SIGTERM，开始关闭..."),
    }

    tokio::time::sleep(std::time::Duration::from_secs(SHUTDOWN_TIMEOUT_SECONDS)).await;
}
