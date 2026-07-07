//! 打扰层（notify）：daemon 主动把"有消息"信号推到 agent 执行环境。
//!
//! 原则：只发信号 + 取信命令模板，绝不注入消息正文。

use crate::identity::session_file::{self, NotifyTarget, SessionFile};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

pub mod plugin;
pub mod tmux;
pub mod zellij;

const DEFAULT_NOTIFY_COOLDOWN_MS: u64 = 1000;

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

/// 按 address 限流，避免消息风暴产生大量插件进程。
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

    /// 检查该 address 是否可以触发 notify。
    /// 若在 cooldown 期内返回 false；否则更新 timestamp 返回 true。
    pub fn should_notify(&self, address: &str) -> bool {
        let now = Instant::now();
        let mut map = self.last.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(address) {
            Some(last) if now.duration_since(*last) < self.cooldown => false,
            _ => {
                map.insert(address.to_string(), now);
                true
            }
        }
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
    fn inject(&self, target: &NotifyTarget, hint: &NotifyHint) -> Result<(), NotifyError>;
}

/// 根据名字获取通道实现。
/// 支持 "zellij" / "tmux" / "plugin:<name>"。
pub fn channel_from_name(name: &str) -> Option<Box<dyn NotifyChannel>> {
    if let Some(plugin_name) = name.strip_prefix("plugin:") {
        return plugin::PluginChannel::new(plugin_name).ok().map(|c| {
            let b: Box<dyn NotifyChannel> = Box::new(c);
            b
        });
    }
    match name {
        "zellij" => Some(Box::new(zellij::ZellijChannel)),
        "tmux" => Some(Box::new(tmux::TmuxChannel)),
        _ => None,
    }
}

/// 自动检测当前终端环境，返回 (channel_name, target)。
///
/// 优先级：zellij > tmux > none。
pub fn auto_detect() -> (String, NotifyTarget) {
    if let (Ok(session), Ok(pane)) = (
        std::env::var("ZELLIJ_SESSION_NAME"),
        std::env::var("ZELLIJ_PANE_ID"),
    ) {
        return ("zellij".to_string(), NotifyTarget::Zellij { session, pane });
    }

    if let Ok(pane) = std::env::var("TMUX_PANE") {
        return ("tmux".to_string(), NotifyTarget::Tmux { pane });
    }
    if std::env::var("TMUX").is_ok() {
        // TMUX_PANE 不一定有，尝试 tmux 自身查询当前 pane
        return (
            "tmux".to_string(),
            NotifyTarget::Tmux {
                pane: "%".to_string(),
            },
        );
    }

    ("none".to_string(), NotifyTarget::None)
}

/// 解析用户输入的 --notify 值。
/// "auto" 走自动检测；"none" 禁用；具体通道名返回对应实现。
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
        if plugin::validate_plugin_name(plugin_name).is_err() {
            tracing::warn!("非法 notify 插件名 '{}', 降级为 none", plugin_name);
            return ("none".to_string(), None, NotifyTarget::None);
        }
        let target = NotifyTarget::Plugin {
            name: plugin_name.to_string(),
        };
        let channel = channel_from_name(raw);
        return (raw.to_string(), channel, target);
    }

    if let Some(channel) = channel_from_name(raw) {
        let target = auto_detect().1; // 复用环境变量定位
        return (raw.to_string(), Some(channel), target);
    }
    // 未知通道降级为 none，不阻塞 join
    ("none".to_string(), None, NotifyTarget::None)
}

/// 触发一次 notify。
///
/// 流程：扫描 `.agtalk/*/` 找到 address 匹配的 session，读 notify 配置，注入提示。
/// 失败只返回错误，由调用方决定是否记录日志。
pub async fn trigger(
    dot_agtalk: &Path,
    to_address: &str,
    from_name: &str,
    limiter: &NotifyLimiter,
) -> Result<(), NotifyError> {
    if !limiter.should_notify(to_address) {
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

    channel.inject(&session.notify_target, &hint)
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
            notify_channel: "zellij".to_string(),
            notify_target: NotifyTarget::Zellij {
                session: "sess".to_string(),
                pane: "1".to_string(),
            },
        };
        session_file::write(&dot, "nora", &session).unwrap();
        let found = find_session_by_address(&dot, "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(found.name, "nora");
    }

    #[test]
    fn auto_detect_zellij() {
        let prev_session = std::env::var_os("ZELLIJ_SESSION_NAME");
        let prev_pane = std::env::var_os("ZELLIJ_PANE_ID");
        let prev_tmux = std::env::var_os("TMUX");
        let prev_tmux_pane = std::env::var_os("TMUX_PANE");

        std::env::remove_var("TMUX");
        std::env::remove_var("TMUX_PANE");
        std::env::set_var("ZELLIJ_SESSION_NAME", "test-session");
        std::env::set_var("ZELLIJ_PANE_ID", "p1");

        let (name, target) = auto_detect();
        assert_eq!(name, "zellij");
        assert_eq!(
            target,
            NotifyTarget::Zellij {
                session: "test-session".to_string(),
                pane: "p1".to_string(),
            }
        );

        if let Some(v) = prev_session {
            std::env::set_var("ZELLIJ_SESSION_NAME", v);
        } else {
            std::env::remove_var("ZELLIJ_SESSION_NAME");
        }
        if let Some(v) = prev_pane {
            std::env::set_var("ZELLIJ_PANE_ID", v);
        } else {
            std::env::remove_var("ZELLIJ_PANE_ID");
        }
        if let Some(v) = prev_tmux {
            std::env::set_var("TMUX", v);
        } else {
            std::env::remove_var("TMUX");
        }
        if let Some(v) = prev_tmux_pane {
            std::env::set_var("TMUX_PANE", v);
        } else {
            std::env::remove_var("TMUX_PANE");
        }
    }

    #[test]
    fn auto_detect_tmux() {
        let prev_session = std::env::var_os("ZELLIJ_SESSION_NAME");
        let prev_pane = std::env::var_os("ZELLIJ_PANE_ID");
        let prev_tmux_pane = std::env::var_os("TMUX_PANE");

        std::env::remove_var("ZELLIJ_SESSION_NAME");
        std::env::remove_var("ZELLIJ_PANE_ID");
        std::env::set_var("TMUX_PANE", "%0");

        let (name, target) = auto_detect();
        assert_eq!(name, "tmux");
        assert_eq!(
            target,
            NotifyTarget::Tmux {
                pane: "%0".to_string(),
            }
        );

        if let Some(v) = prev_session {
            std::env::set_var("ZELLIJ_SESSION_NAME", v);
        } else {
            std::env::remove_var("ZELLIJ_SESSION_NAME");
        }
        if let Some(v) = prev_pane {
            std::env::set_var("ZELLIJ_PANE_ID", v);
        } else {
            std::env::remove_var("ZELLIJ_PANE_ID");
        }
        if let Some(v) = prev_tmux_pane {
            std::env::set_var("TMUX_PANE", v);
        } else {
            std::env::remove_var("TMUX_PANE");
        }
    }

    #[test]
    fn auto_detect_none() {
        let prev_session = std::env::var_os("ZELLIJ_SESSION_NAME");
        let prev_pane = std::env::var_os("ZELLIJ_PANE_ID");
        let prev_tmux = std::env::var_os("TMUX");
        let prev_tmux_pane = std::env::var_os("TMUX_PANE");

        std::env::remove_var("ZELLIJ_SESSION_NAME");
        std::env::remove_var("ZELLIJ_PANE_ID");
        std::env::remove_var("TMUX");
        std::env::remove_var("TMUX_PANE");

        let (name, target) = auto_detect();
        assert_eq!(name, "none");
        assert_eq!(target, NotifyTarget::None);

        if let Some(v) = prev_session {
            std::env::set_var("ZELLIJ_SESSION_NAME", v);
        } else {
            std::env::remove_var("ZELLIJ_SESSION_NAME");
        }
        if let Some(v) = prev_pane {
            std::env::set_var("ZELLIJ_PANE_ID", v);
        } else {
            std::env::remove_var("ZELLIJ_PANE_ID");
        }
        if let Some(v) = prev_tmux {
            std::env::set_var("TMUX", v);
        } else {
            std::env::remove_var("TMUX");
        }
        if let Some(v) = prev_tmux_pane {
            std::env::set_var("TMUX_PANE", v);
        } else {
            std::env::remove_var("TMUX_PANE");
        }
    }

    #[test]
    fn resolve_channel_plugin() {
        let (name, _channel, target) = resolve_channel("plugin:macos");
        assert_eq!(name, "plugin:macos");
        assert_eq!(
            target,
            NotifyTarget::Plugin {
                name: "macos".to_string(),
            }
        );
    }

    #[test]
    fn resolve_channel_unknown_becomes_none() {
        let (name, channel, target) = resolve_channel("webhook");
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
    fn limiter_skips_duplicate_within_cooldown() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        assert!(limiter.should_notify("a"));
        assert!(!limiter.should_notify("a"));
    }

    #[test]
    fn limiter_allows_after_cooldown() {
        let limiter = NotifyLimiter::new(Duration::from_millis(10));
        assert!(limiter.should_notify("a"));
        assert!(!limiter.should_notify("a"));
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.should_notify("a"));
    }

    #[test]
    fn limiter_per_address_isolated() {
        let limiter = NotifyLimiter::new(Duration::from_millis(1000));
        assert!(limiter.should_notify("a"));
        assert!(limiter.should_notify("b"));
        assert!(!limiter.should_notify("a"));
    }
}
