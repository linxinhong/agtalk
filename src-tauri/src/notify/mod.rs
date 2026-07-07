//! 打扰层（notify）：daemon 主动把"有消息"信号推到 agent 执行环境。
//!
//! 原则：只发信号 + 取信命令模板，绝不注入消息正文。
//!
//! v2 架构：agtalk core 只负责调用外部 notify plugin 的 `discover`/`send` 子命令。
//! zellij/tmux 等具体实现已迁出 core，作为独立可执行插件。

use crate::identity::session_file::{self, NotifyTarget, SessionFile};
use crate::notify::plugin::{PluginChannel, PluginEndpoint};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

pub mod plugin;

const DEFAULT_NOTIFY_COOLDOWN_MS: u64 = 1000;
/// auto 检测时尝试的 plugin 通道，按优先级排序。
const AUTO_PLUGIN_CANDIDATES: &[&str] = &["zellij", "tmux"];

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("身份错误: {0}")]
    Identity(#[from] crate::identity::IdentityError),
    #[error("未找到目标 session")]
    SessionNotFound,
    #[error("notify 通道未启用")]
    ChannelDisabled,
    #[error("限流中")]
    RateLimited,
    #[error("外部命令失败: {0}")]
    CommandFailed(String),
    #[error("{0}")]
    Other(String),
}

/// 按 address 限流 notify 信号的成功触发，避免消息风暴产生大量插件进程/终端注入。
///
/// 使用方式：
/// - `check(address)`：只读检查，返回 true 表示不在 cooldown 期内。
/// - 执行 notify 注入。
/// - `record(address)`：仅在注入成功后调用，更新 cooldown 时间戳。
pub struct NotifyLimiter {
    cooldown: Duration,
    last: Mutex<HashMap<String, Instant>>,
}

impl NotifyLimiter {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            last: Mutex::new(HashMap::new()),
        }
    }

    /// 默认 1000ms cooldown。
    pub fn default_cooldown() -> Self {
        Self::new(Duration::from_millis(DEFAULT_NOTIFY_COOLDOWN_MS))
    }

    /// 只读检查该 address 是否可以触发 notify。
    /// 若在 cooldown 期内返回 false；否则返回 true。不更新 timestamp。
    pub fn check(&self, address: &str) -> bool {
        let now = Instant::now();
        let map = self.last.lock().unwrap_or_else(|e| e.into_inner());
        !matches!(
            map.get(address),
            Some(last) if now.duration_since(*last) < self.cooldown
        )
    }

    /// 在 notify 成功注入后调用，更新该 address 的 cooldown 时间戳。
    pub fn record(&self, address: &str) {
        let now = Instant::now();
        let mut map = self.last.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(address.to_string(), now);
    }
}

/// 传给通道的提示信息。
pub struct NotifyHint {
    pub from_name: String,
    pub binary_path: String,
    pub workspace: String,
    pub agent_name: String,
    pub agent_address: String,
}

/// notify 通道抽象。
pub trait NotifyChannel: Send + Sync {
    fn name(&self) -> &'static str;
    /// 调用插件 discover，返回当前可用 endpoint。
    fn discover(&self) -> Result<Option<PluginEndpoint>, NotifyError>;
    /// 执行提醒（或 dry-run 验证）。
    fn send(
        &self,
        target: &NotifyTarget,
        hint: &NotifyHint,
        dry_run: bool,
    ) -> Result<(), NotifyError>;
}

/// 根据名字获取通道实现。
/// 支持 "plugin:<name>" / "none"。
pub fn channel_from_name(name: &str) -> Option<Box<dyn NotifyChannel>> {
    if let Some(plugin_name) = name.strip_prefix("plugin:") {
        return PluginChannel::new(plugin_name).ok().map(|c| {
            let b: Box<dyn NotifyChannel> = Box::new(c);
            b
        });
    }
    None
}

/// 自动检测当前环境，返回第一个 ready 的 plugin 通道。
///
/// 依次尝试 plugin:zellij、plugin:tmux；都不可用则返回 ("none", NotifyTarget::None)。
pub fn auto_detect() -> (String, NotifyTarget) {
    for candidate in AUTO_PLUGIN_CANDIDATES {
        let channel_name = format!("plugin:{}", candidate);
        let Some(channel) = channel_from_name(&channel_name) else {
            continue;
        };
        match channel.discover() {
            Ok(Some(endpoint)) if endpoint.ready => {
                return (
                    channel_name,
                    NotifyTarget::Plugin {
                        name: candidate.to_string(),
                        endpoint: endpoint.endpoint,
                    },
                );
            }
            _ => continue,
        }
    }
    ("none".to_string(), NotifyTarget::None)
}

/// 解析用户输入的 --notify 值。
/// "auto" 走自动检测；"none" 禁用；plugin:<name> 返回对应实现。
pub fn resolve_channel(raw: &str) -> (String, Option<Box<dyn NotifyChannel>>, NotifyTarget) {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("none") {
        return ("none".to_string(), None, NotifyTarget::None);
    }
    if raw.eq_ignore_ascii_case("auto") {
        let (name, target) = auto_detect();
        let channel = channel_from_name(&name);
        return (name, channel, target);
    }
    if let Some(plugin_name) = raw.strip_prefix("plugin:") {
        if PluginChannel::new(plugin_name).is_err() {
            tracing::warn!("非法 notify 插件名 '{}', 降级为 none", plugin_name);
            return ("none".to_string(), None, NotifyTarget::None);
        }
        let target = NotifyTarget::Plugin {
            name: plugin_name.to_string(),
            endpoint: serde_json::Value::Null,
        };
        let channel = channel_from_name(raw);
        return (raw.to_string(), channel, target);
    }
    // 未知通道降级为 none，不阻塞 join。
    tracing::warn!("未知 notify 通道 '{}', 降级为 none", raw);
    ("none".to_string(), None, NotifyTarget::None)
}

/// 触发一次 notify。
///
/// 流程：扫描 `.agtalk/*/` 找到 address 匹配的 session，读 notify 配置，注入提示。
/// 只有在注入成功后才会记录 cooldown；session 缺失、channel disabled、插件失败等都不消耗 cooldown。
/// 对 plugin 通道，send 失败时会自动重新 discover 并刷新 endpoint 重试一次。
/// 失败只返回错误，由调用方决定是否记录日志。
pub async fn trigger(
    dot_agtalk: &Path,
    to_address: &str,
    from_name: &str,
    limiter: &NotifyLimiter,
) -> Result<(), NotifyError> {
    if !limiter.check(to_address) {
        return Err(NotifyError::RateLimited);
    }

    let session = find_session_by_address(dot_agtalk, to_address)?;

    if session.notify_channel.eq_ignore_ascii_case("none") || session.notify_channel.is_empty() {
        return Err(NotifyError::ChannelDisabled);
    }

    let channel = channel_from_name(&session.notify_channel).ok_or_else(|| {
        NotifyError::Other(format!("未知 notify 通道: {}", session.notify_channel))
    })?;

    let binary_path = current_binary_path();
    let hint = NotifyHint {
        from_name: from_name.to_string(),
        binary_path,
        workspace: session.workspace.clone(),
        agent_name: session.name.clone(),
        agent_address: session.address.clone(),
    };

    // 第一次尝试。
    if let Err(e) = channel.send(&session.notify_target, &hint, false) {
        // plugin 通道失败时尝试刷新 endpoint 重试一次。
        if let Some(plugin) = channel_from_name(&session.notify_channel) {
            if let Ok(Some(endpoint)) = plugin.discover() {
                if endpoint.ready {
                    let refreshed = NotifyTarget::Plugin {
                        name: session
                            .notify_target
                            .plugin_name()
                            .unwrap_or_default()
                            .to_string(),
                        endpoint: endpoint.endpoint,
                    };
                    channel.send(&refreshed, &hint, false)?;
                    // 写回 session.json。
                    let mut updated = session.clone();
                    updated.notify_target = refreshed;
                    let _ = session_file::write(dot_agtalk, &session.name, &updated);
                    limiter.record(to_address);
                    return Ok(());
                }
            }
        }
        return Err(e);
    }

    limiter.record(to_address);
    Ok(())
}

/// 构造注入文本。不包含正文，只含信号 + 取信命令模板。
pub fn build_hint_text(hint: &NotifyHint) -> String {
    format!(
        "[agtalk] 新消息来自 {}，运行 {} msg read 查看\n",
        hint.from_name, hint.binary_path
    )
}

fn current_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "agtalk".to_string())
}

fn find_session_by_address(dot_agtalk: &Path, address: &str) -> Result<SessionFile, NotifyError> {
    if !dot_agtalk.exists() {
        return Err(NotifyError::SessionNotFound);
    }

    for entry in std::fs::read_dir(dot_agtalk)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let session = match session_file::read(dot_agtalk, &name) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if session.address == address {
            return Ok(session);
        }
    }

    Err(NotifyError::SessionNotFound)
}

/// 执行外部命令，参数数组形式（不经 shell）。
pub fn run_command(cmd: &str, args: &[&str]) -> Result<(), NotifyError> {
    let output = Command::new(cmd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NotifyError::CommandFailed(format!(
            "{} {:?} failed: {}",
            cmd, args, stderr
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::session_file;
    use tempfile::TempDir;

    #[test]
    fn build_hint_does_not_include_body() {
        let hint = NotifyHint {
            from_name: "nora".to_string(),
            binary_path: "/usr/local/bin/agtalk".to_string(),
            workspace: "projA".to_string(),
            agent_name: "codex".to_string(),
            agent_address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let text = build_hint_text(&hint);
        assert!(text.contains("nora"));
        assert!(text.contains("/usr/local/bin/agtalk msg read"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn find_session_by_address_works() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "nora".to_string(),
            workspace: "projA".to_string(),
            intro: "前端".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            command: "agtalk".to_string(),
            notify_channel: "plugin:zellij".to_string(),
            notify_target: NotifyTarget::Plugin {
                name: "zellij".to_string(),
                endpoint: serde_json::json!({ "session": "sess", "pane": "1" }),
            },
        };
        session_file::write(&dot, "nora", &session).unwrap();
        let found = find_session_by_address(&dot, "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(found.name, "nora");
    }

    #[test]
    fn resolve_channel_none() {
        let (name, channel, target) = resolve_channel("none");
        assert_eq!(name, "none");
        assert!(channel.is_none());
        assert_eq!(target, NotifyTarget::None);
    }

    #[test]
    fn resolve_channel_plugin() {
        let (name, _channel, target) = resolve_channel("plugin:macos");
        assert_eq!(name, "plugin:macos");
        assert_eq!(
            target,
            NotifyTarget::Plugin {
                name: "macos".to_string(),
                endpoint: serde_json::Value::Null,
            }
        );
    }

    #[test]
    fn resolve_channel_unknown_becomes_none() {
        let (name, channel, target) = resolve_channel("zellij");
        assert_eq!(name, "none");
        assert!(channel.is_none());
        assert_eq!(target, NotifyTarget::None);
    }

    #[test]
    fn resolve_channel_rejects_invalid_plugin_name() {
        let (name, channel, target) = resolve_channel("plugin:foo/bar");
        assert_eq!(name, "none");
        assert!(channel.is_none());
        assert_eq!(target, NotifyTarget::None);
    }

    #[test]
    fn limiter_check_does_not_update_timestamp() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
    }

    #[test]
    fn limiter_record_enables_cooldown() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        assert!(limiter.check("a"));
        limiter.record("a");
        assert!(!limiter.check("a"));
    }

    #[test]
    fn limiter_failed_attempt_does_not_consume_cooldown() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        limiter.record("a");
        assert!(!limiter.check("a"));
    }

    #[test]
    fn limiter_allows_after_cooldown() {
        let limiter = NotifyLimiter::new(Duration::from_millis(10));
        assert!(limiter.check("a"));
        limiter.record("a");
        assert!(!limiter.check("a"));
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.check("a"));
    }

    #[test]
    fn limiter_per_address_isolated() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        limiter.record("a");
        assert!(!limiter.check("a"));
        assert!(limiter.check("b"));
    }
}
