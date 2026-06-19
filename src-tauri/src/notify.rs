//! Agent 提醒插件系统：zellij / tmux 内置插件 + 外部命令插件。

use crate::config::{NotifyConfig, NotifyPluginEntry};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// 目标 pane 的端点信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifyEndpoint {
    pub session: String,
    pub pane_id: String,
}

/// 提醒插件执行时需要的上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyContext {
    pub message_id: String,
    pub short_message_id: String,
    pub from: String,
    pub to: String,
    pub text: String,
    pub command: String,
    pub send_enter: bool,
    pub endpoint: serde_json::Value,
}

/// 单个提醒插件
#[async_trait]
pub trait NotifyPlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn notify(&self, ctx: &NotifyContext) -> Result<()>;
}

/// 插件注册表
pub struct NotifyPluginRegistry {
    plugins: HashMap<String, Arc<dyn NotifyPlugin>>,
}

impl Default for NotifyPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl NotifyPluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn from_config(cfg: &NotifyConfig) -> Self {
        let mut registry = Self::new();
        for (name, entry) in &cfg.plugins {
            match entry {
                NotifyPluginEntry::Builtin => match name.as_str() {
                    "zellij" => registry.register(Arc::new(ZellijPlugin)),
                    "tmux" => registry.register(Arc::new(TmuxPlugin)),
                    other => {
                        tracing::warn!("未知内置 notify 插件: {}", other);
                    }
                },
                NotifyPluginEntry::Command { path, timeout_ms } => {
                    let path = PathBuf::from(path);
                    if !path.is_absolute() {
                        tracing::warn!("外部 notify 插件路径必须是绝对路径: {:?}", path);
                        continue;
                    }
                    registry.register(Arc::new(CommandPlugin {
                        name: name.clone(),
                        path: path.to_string_lossy().to_string(),
                        timeout_ms: timeout_ms.unwrap_or(2000),
                    }));
                }
            }
        }
        registry
    }

    pub fn register(&mut self, plugin: Arc<dyn NotifyPlugin>) {
        self.plugins.insert(plugin.name().to_string(), plugin);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn NotifyPlugin>> {
        self.plugins.get(name).cloned()
    }
}

/// 标准提醒文本
pub fn build_notify_text(short_id: &str) -> String {
    format!("[agtalk:{}] | exec agtalk detail -", short_id)
}

pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// 从 notify_config JSON 中读取 notify 配置
#[derive(Debug, Clone, Deserialize)]
pub struct NotifyTransportConfig {
    pub plugin: String,
    pub endpoint: serde_json::Value,
    #[serde(default = "default_send_enter")]
    pub send_enter: bool,
}

fn default_send_enter() -> bool {
    true
}

/// 构造 session 级别 notify_config（直接存到 agent_sessions.notify_config）
pub fn build_notify_config(
    plugin: &str,
    endpoint: &NotifyEndpoint,
    send_enter: bool,
) -> serde_json::Value {
    serde_json::json!({
        "plugin": plugin,
        "endpoint": {
            "session": endpoint.session,
            "pane_id": endpoint.pane_id,
        },
        "send_enter": send_enter,
        "captured_by": "join",
    })
}

// ── 内置：zellij ──────────────────────────────────────

pub struct ZellijPlugin;

#[async_trait]
impl NotifyPlugin for ZellijPlugin {
    fn name(&self) -> &str {
        "zellij"
    }

    async fn notify(&self, ctx: &NotifyContext) -> Result<()> {
        let endpoint: NotifyEndpoint =
            serde_json::from_value(ctx.endpoint.clone()).context("zellij endpoint 解析失败")?;
        let text = build_notify_text(&ctx.short_message_id);
        let session = endpoint.session.clone();
        let pane_id = endpoint.pane_id.clone();

        // 1. 写入提醒文本
        let write_args = vec![
            "--session".to_string(),
            session.clone(),
            "action".to_string(),
            "write-chars".to_string(),
            "--pane-id".to_string(),
            pane_id.clone(),
            text,
        ];
        run_command("zellij", &write_args, 2000).await?;

        // 2. 如果需要，再发送 Enter 键
        if ctx.send_enter {
            let enter_args = vec![
                "--session".to_string(),
                session,
                "action".to_string(),
                "send-keys".to_string(),
                "--pane-id".to_string(),
                pane_id,
                "Enter".to_string(),
            ];
            run_command("zellij", &enter_args, 2000).await?;
        }

        Ok(())
    }
}

/// 检测当前是否在 zellij pane 中
pub fn detect_zellij_endpoint() -> Option<NotifyEndpoint> {
    let session = std::env::var("ZELLIJ_SESSION_NAME").ok()?;
    let pane_id = std::env::var("ZELLIJ_PANE_ID").ok()?;
    Some(NotifyEndpoint { session, pane_id })
}

// ── 内置：tmux ────────────────────────────────────────

pub struct TmuxPlugin;

#[async_trait]
impl NotifyPlugin for TmuxPlugin {
    fn name(&self) -> &str {
        "tmux"
    }

    async fn notify(&self, ctx: &NotifyContext) -> Result<()> {
        let endpoint: NotifyEndpoint =
            serde_json::from_value(ctx.endpoint.clone()).context("tmux endpoint 解析失败")?;
        let text = build_notify_text(&ctx.short_message_id);
        let target = format!("{}:{}", endpoint.session, endpoint.pane_id);
        let mut args = vec!["send-keys".to_string(), "-t".to_string(), target, text];
        if ctx.send_enter {
            args.push("Enter".to_string());
        }
        run_command("tmux", &args, 2000).await
    }
}

fn tmux_session_name() -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#S"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// 检测当前是否在 tmux pane 中
pub fn detect_tmux_endpoint() -> Option<NotifyEndpoint> {
    let pane_id = std::env::var("TMUX_PANE").ok()?;
    let session = tmux_session_name()?;
    Some(make_tmux_endpoint(&pane_id, &session))
}

/// 纯函数：用给定的 pane / session 构造 tmux endpoint，便于单测
pub fn make_tmux_endpoint(pane_id: &str, session: &str) -> NotifyEndpoint {
    NotifyEndpoint {
        session: session.to_string(),
        pane_id: pane_id.to_string(),
    }
}

// ── 外部命令插件 ──────────────────────────────────────

pub struct CommandPlugin {
    name: String,
    path: String,
    timeout_ms: u64,
}

#[async_trait]
impl NotifyPlugin for CommandPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    async fn notify(&self, ctx: &NotifyContext) -> Result<()> {
        let payload = serde_json::to_string(ctx).context("序列化 notify payload 失败")?;
        let path = self.path.clone();
        let timeout = Duration::from_millis(self.timeout_ms);
        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                std::process::Command::new(&path)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(stdin) = child.stdin.take() {
                            let mut stdin = stdin;
                            let _ = stdin.write_all(payload.as_bytes());
                        }
                        child.wait()
                    })
            }),
        )
        .await;
        match result {
            Ok(Ok(Ok(status))) if status.success() => Ok(()),
            Ok(Ok(Ok(status))) => bail!("外部插件退出码非零: {}", status),
            Ok(Ok(Err(e))) => bail!("运行外部插件失败: {}", e),
            Ok(Err(e)) => bail!("执行外部插件任务失败: {}", e),
            Err(_) => bail!("外部插件执行超时"),
        }
    }
}

// ── 工具：异步执行外部命令 ────────────────────────────

pub async fn run_command(program: &str, args: &[String], timeout_ms: u64) -> Result<()> {
    let program = program.to_string();
    let args = args.to_vec();
    let timeout = Duration::from_millis(timeout_ms);
    let program_for_error = program.clone();
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            std::process::Command::new(&program)
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
        }),
    )
    .await;
    match result {
        Ok(Ok(Ok(output))) if output.status.success() => Ok(()),
        Ok(Ok(Ok(output))) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("{} 失败 ({}): {}", program_for_error, output.status, stderr)
        }
        Ok(Ok(Err(e))) => bail!("无法启动 {}: {}", program_for_error, e),
        Ok(Err(e)) => bail!("执行 {} 任务失败: {}", program_for_error, e),
        Err(_) => bail!("{} 执行超时", program_for_error),
    }
}

// ── 测试 ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_notify_text() {
        assert_eq!(
            build_notify_text("7a8b9335"),
            "[agtalk:7a8b9335] | exec agtalk detail -"
        );
    }

    #[test]
    fn test_short_id() {
        assert_eq!(short_id("7a8b9335-1234-1234-1234-123456789abc"), "7a8b9335");
    }

    #[test]
    fn test_detect_zellij_endpoint() {
        let session = std::env::var("ZELLIJ_SESSION_NAME").ok();
        let pane = std::env::var("ZELLIJ_PANE_ID").ok();
        match (session, pane) {
            (Some(session), Some(pane)) => {
                let ep = detect_zellij_endpoint().expect("应检测到 zellij endpoint");
                assert_eq!(ep.session, session);
                assert_eq!(ep.pane_id, pane);
            }
            _ => assert!(detect_zellij_endpoint().is_none()),
        }
    }

    #[test]
    fn test_detect_tmux_endpoint_without_env() {
        assert!(detect_tmux_endpoint().is_none());
    }

    #[test]
    fn test_build_notify_config() {
        let ep = NotifyEndpoint {
            session: "sess".to_string(),
            pane_id: "pane-1".to_string(),
        };
        let cfg = build_notify_config("zellij", &ep, true);
        assert_eq!(cfg.get("plugin").and_then(|v| v.as_str()), Some("zellij"));
        assert_eq!(cfg.get("send_enter").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            cfg.get("captured_by").and_then(|v| v.as_str()),
            Some("join")
        );
    }

    #[test]
    fn test_notify_transport_config_parsing() {
        let notify = serde_json::json!({
            "plugin": "tmux",
            "endpoint": { "session": "s", "pane_id": "%0" },
            "send_enter": false,
            "captured_by": "join"
        });
        let parsed: NotifyTransportConfig = serde_json::from_value(notify).unwrap();
        assert_eq!(parsed.plugin, "tmux");
        assert!(!parsed.send_enter);
    }

    #[test]
    fn test_registry_from_config_skips_relative_command() {
        let mut cfg = NotifyConfig::default();
        cfg.plugins.insert(
            "bad".to_string(),
            NotifyPluginEntry::Command {
                path: "./relative".to_string(),
                timeout_ms: None,
            },
        );
        let registry = NotifyPluginRegistry::from_config(&cfg);
        assert!(registry.get("bad").is_none());
        assert!(registry.get("zellij").is_some());
        assert!(registry.get("tmux").is_some());
    }
}
