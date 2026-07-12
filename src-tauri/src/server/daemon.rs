//! daemon 生命周期：启动、停止、状态、单例锁。

use crate::config::AgConfig;
use crate::identity::mailbox;
use crate::paths::{daemon_pid_path, daemon_status_path, set_permissions_0600};
use crate::proto::ServerMsg;
use crate::routing::inbox;
use crate::server::http::routes;
use crate::server::state::AppState;
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
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

impl From<serde_json::Error> for DaemonError {
    fn from(e: serde_json::Error) -> Self {
        DaemonError::Io(std::io::Error::other(e.to_string()))
    }
}

/// daemon.json 持久化内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatusFile {
    pub pid: u32,
    pub start_time: u64,
    pub version: String,
    pub http_port: u16,
    pub config_path: String,
    pub db_path: String,
}

pub async fn start(dot_agtalk: PathBuf) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        use nix::unistd::setsid;
        let _ = setsid();
    }

    // tracing 默认写 stdout（daemon 启动时 stdout 已重定向到 /dev/null），
    // 显式指定 stderr 才能落进 daemon.log
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init();

    // rustls 0.23 未启用默认 CryptoProvider（tokio-tungstenite 的 rustls-tls-webpki-roots
    // 不带 provider），飞书长连接首次握手前必须显式安装，否则 panic
    let _ = rustls::crypto::ring::default_provider().install_default();

    if is_running() {
        return Err(DaemonError::AlreadyRunning);
    }

    let config = AgConfig::load()?;
    let storage = Storage::open()?;

    // 确保全局用户 memory 目录存在
    if let Err(e) = crate::paths::ensure_global_memory_dir() {
        error!("创建全局 memory 目录失败: {}", e);
    }

    // 确保 human mailbox 存在，并为本机 human 客户端颁发 session（token 存文件，不落 agent 记忆）
    match mailbox::ensure_human(&storage, &config.human) {
        Ok(human_address) => {
            if let Err(e) =
                crate::identity::human_session::ensure(&human_address, &config.human.name)
            {
                error!("创建 human session 失败: {}", e);
            }
            // delivery 可恢复性：补齐历史 fanout 失败/缺行的 delivery 记录
            match crate::human::reconcile(&storage, &config.human) {
                Ok(n) if n > 0 => info!("human delivery reconcile 补齐 {} 行", n),
                Ok(_) => {}
                Err(e) => error!("human delivery reconcile 失败: {}", e),
            }
        }
        Err(e) => error!("创建 human mailbox 失败: {}", e),
    }

    write_pid_file()?;

    let mut state = AppState::new(storage, config.clone(), dot_agtalk);
    // daemon 环境启用桌面弹窗投递（测试保持 disabled，不拉起真实子进程）
    state.popup = Arc::new(crate::human::popup::PopupTransport::enabled());

    // 飞书 surface：enabled 时 spawn 全局 Router（单条长连接，入站 receipt 幂等）
    if config.feishu.enabled {
        state.feishu = std::sync::Arc::new(crate::feishu::dispatch::FeishuDispatcher::enabled(
            config.feishu.clone(),
        ));
        let router = std::sync::Arc::new(crate::feishu::router::FeishuRouter::new(
            state.storage.clone(),
            config.feishu.clone(),
            state.feishu_link.clone(),
            state.registry.clone(),
            state.notify_limiter.clone(),
            state.dot_agtalk.clone(),
        ));
        tokio::spawn(router.run());
        info!("feishu surface 已启用，Router 已启动");
    }

    let app = routes(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], config.http_port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(DaemonError::Bind)?;
    info!("daemon 监听 127.0.0.1:{}", config.http_port);

    let start_time = now_unix_secs();
    write_status_file(
        std::process::id(),
        start_time,
        config.http_port,
        env!("CARGO_PKG_VERSION"),
        &crate::paths::config_path()?,
        &crate::paths::db_path()?,
    )?;

    let server = axum::serve(listener, app);
    let shutdown = server.with_graceful_shutdown(shutdown_signal());

    if let Err(e) = shutdown.await {
        error!("server 异常退出: {}", e);
    }

    remove_pid_file();
    remove_status_file();
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
        remove_status_file();
        return Err(DaemonError::NotRunning);
    }

    Ok(())
}

pub fn status() -> String {
    match is_running() {
        true => {
            if let Ok(pid) = read_pid_file() {
                format!("running (pid {})", pid)
            } else {
                "running".to_string()
            }
        }
        false => {
            remove_status_file();
            "stopped".to_string()
        }
    }
}

/// 读取 daemon.json；若文件不存在或 PID 已死，返回 None 并清理残留。
pub fn read_status_file() -> Result<Option<DaemonStatusFile>, DaemonError> {
    let path = daemon_status_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let file: DaemonStatusFile = serde_json::from_str(&content).map_err(|e| {
        DaemonError::Io(std::io::Error::other(format!(
            "daemon.json 解析失败: {}",
            e
        )))
    })?;

    let mut sys = System::new_all();
    sys.refresh_processes();
    if sys.process(Pid::from(file.pid as usize)).is_none() {
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }

    Ok(Some(file))
}

pub fn is_running() -> bool {
    let pid = match read_pid_file() {
        Ok(p) if p > 0 => p,
        _ => return false,
    };
    let mut sys = System::new_all();
    sys.refresh_processes();
    sys.process(Pid::from(pid as usize)).is_some()
}

/// 构造 daemon 实时状态响应（HTTP handler 使用）。
pub fn build_status(state: &AppState) -> ServerMsg {
    let file = match read_status_file() {
        Ok(Some(f)) => f,
        _ => {
            return ServerMsg::Error {
                code: "daemon_not_running".into(),
                message: "daemon 未运行".into(),
            }
        }
    };

    let active_mailboxes = mailbox::count_active(&state.storage).unwrap_or(0);
    let pending_messages = inbox::count_pending(&state.storage).unwrap_or(0);
    let sse_subscribers = state.registry.subscriber_count();
    let uptime_seconds = now_unix_secs().saturating_sub(file.start_time);

    ServerMsg::DaemonStatus {
        pid: file.pid,
        start_time: file.start_time,
        version: file.version,
        http_port: file.http_port,
        uptime_seconds,
        active_mailboxes,
        pending_messages,
        sse_subscribers,
        config_path: file.config_path,
        db_path: file.db_path,
    }
}

pub(crate) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

pub(crate) fn write_status_file(
    pid: u32,
    start_time: u64,
    http_port: u16,
    version: &str,
    config_path: &std::path::Path,
    db_path: &std::path::Path,
) -> Result<(), DaemonError> {
    let path = daemon_status_path()?;
    let file = DaemonStatusFile {
        pid,
        start_time,
        version: version.to_string(),
        http_port,
        config_path: config_path.to_string_lossy().to_string(),
        db_path: db_path.to_string_lossy().to_string(),
    };
    let content = serde_json::to_string_pretty(&file)?;
    std::fs::write(&path, content)?;
    set_permissions_0600(&path)?;
    Ok(())
}

pub(crate) fn remove_status_file() {
    if let Ok(path) = daemon_status_path() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::server::state::AppState;
    use crate::storage::Storage;
    use std::ffi::OsString;

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

    fn test_guard() -> (EnvGuard, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        (EnvGuard::set(tmp.path()), tmp)
    }

    #[test]
    fn status_file_roundtrip() {
        let (guard, _tmp) = test_guard();
        let cur_pid = std::process::id();
        let file = DaemonStatusFile {
            pid: cur_pid,
            start_time: 1_000_000,
            version: "0.1.0".to_string(),
            http_port: 19527,
            config_path: "/tmp/config.json".to_string(),
            db_path: "/tmp/agtalk.db".to_string(),
        };
        write_status_file(
            file.pid,
            file.start_time,
            file.http_port,
            &file.version,
            std::path::Path::new(&file.config_path),
            std::path::Path::new(&file.db_path),
        )
        .unwrap();

        let read = read_status_file().unwrap().unwrap();
        assert_eq!(read.pid, file.pid);
        assert_eq!(read.start_time, file.start_time);
        assert_eq!(read.http_port, file.http_port);
        assert_eq!(read.version, file.version);

        remove_status_file();
        assert!(read_status_file().unwrap().is_none());
        drop(guard);
    }

    #[test]
    fn build_status_returns_live_metrics() {
        let (guard, _tmp) = test_guard();
        let storage = Storage::open_in_memory().unwrap();
        let _nora = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
        let state = AppState::new(
            storage,
            AgConfig::default(),
            std::path::PathBuf::from("/tmp"),
        );

        write_status_file(
            std::process::id(),
            now_unix_secs(),
            19527,
            "0.1.0",
            std::path::Path::new("/tmp/config.json"),
            std::path::Path::new("/tmp/agtalk.db"),
        )
        .unwrap();

        let status = build_status(&state);
        match status {
            ServerMsg::DaemonStatus {
                active_mailboxes,
                pending_messages,
                ..
            } => {
                assert_eq!(active_mailboxes, 1);
                assert_eq!(pending_messages, 0);
            }
            other => panic!("expected DaemonStatus, got {:?}", other),
        }

        remove_status_file();
        drop(guard);
    }
}
