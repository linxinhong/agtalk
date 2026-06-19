//! agtalk — 单一二进制入口。
//!
//! 第一个参数分派：
//!   agtalk <收件人> <正文>   → CLI 发送消息
//!   agtalk daemon start      → 启动后台 daemon
//!   agtalk --settings        → GUI 设置（Tauri 模式）
//!   agtalk gui               → 启动 Tauri GUI
//!   agtalk __daemon           → 隐藏入口：daemon 进程（由 daemon start spawn）

mod cli;
mod commands;
mod config;
mod identity;
mod ipc;
mod join_plugin;
mod notify;
mod paths;
mod server;
mod session;
mod storage;
#[cfg(test)]
mod tests;
mod transport;
mod workspace;

use std::sync::Arc;

fn main() {
    let argv: Vec<String> = std::env::args().collect();

    // 隐藏的 daemon 进程入口（由 agtalk daemon start spawn）
    if argv.len() >= 2 && argv[1] == "__daemon" {
        run_daemon();
        return;
    }

    // Tauri GUI 模式
    if argv.len() >= 2 && argv[1] == "gui" {
        agtalk_app::run_gui();
        return;
    }

    // 审批弹窗模式（由 daemon 的 PopupTransport spawn）
    if argv.len() >= 3 && argv[1] == "__popup" {
        agtalk_app::run_popup(argv[2].clone());
        return;
    }

    // 分发到 CLI
    cli::dispatch::dispatch(&argv);
}

fn run_daemon() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // 写入 PID 文件
    let pid_path = paths::pid_path();
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&pid_path, std::process::id().to_string());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("无法创建 tokio runtime");

    let config = Arc::new(config::AgConfig::load().unwrap_or_default());
    let storage =
        Arc::new(storage::Storage::open_with_config(config.clone()).expect("无法打开数据库"));
    if let Err(e) = storage.ensure_human_participant() {
        tracing::warn!("无法创建默认人类参与者 human: {}", e);
    }
    let mut registry = transport::TransportRegistry::new();
    registry.register(Arc::new(transport::TerminalTransport::new()));
    registry.register(Arc::new(transport::PopupTransport::new()));
    let transports = Arc::new(registry);

    let notify_plugins = Arc::new(notify::NotifyPluginRegistry::from_config(&config.notify));

    let socket = paths::socket_path();

    tracing::info!("daemon 启动: {}", socket);
    rt.block_on(async {
        if let Err(e) = server::run(&socket, storage, transports, notify_plugins).await {
            tracing::error!("daemon 异常退出: {}", e);
        }
    });

    // 清理 PID 和 socket
    let _ = std::fs::remove_file(&pid_path);
    let _ = std::fs::remove_file(&socket);
}
