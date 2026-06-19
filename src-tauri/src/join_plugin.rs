//! Join 生命周期插件：在当前 CLI 进程里执行 join 成功后的动作。
//!
//! 与 daemon 中的 notify plugin 不同，这里只处理当前终端环境相关副作用，
//! 例如重命名 zellij pane、tmux window 等。

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;

/// join 成功后传给插件的上下文。
#[allow(dead_code)]
pub struct JoinContext {
    pub name: String,
    pub role: String,
    pub session_id: String,
    pub workspace_root: String,
}

#[async_trait]
pub trait JoinPlugin: Send + Sync {
    fn name(&self) -> &str;
    async fn on_join(&self, ctx: &JoinContext) -> Result<()>;
}

/// 默认插件列表。
pub fn default_plugins() -> Vec<Arc<dyn JoinPlugin>> {
    vec![
        Arc::new(ZellijPaneRenamePlugin),
        Arc::new(TmuxWindowRenamePlugin),
    ]
}

/// 依次执行所有插件，单个失败不影响其他插件和 join 结果。
pub async fn run_all(plugins: &[Arc<dyn JoinPlugin>], ctx: &JoinContext) {
    for plugin in plugins {
        if let Err(e) = plugin.on_join(ctx).await {
            tracing::warn!("join plugin {} 执行失败: {}", plugin.name(), e);
        }
    }
}

// ── 内置：zellij pane 重命名 ───────────────────────────

pub struct ZellijPaneRenamePlugin;

#[async_trait]
impl JoinPlugin for ZellijPaneRenamePlugin {
    fn name(&self) -> &str {
        "zellij-rename-pane"
    }

    async fn on_join(&self, ctx: &JoinContext) -> Result<()> {
        let session = std::env::var("ZELLIJ_SESSION_NAME")
            .context("当前不在 zellij session 中，跳过 pane 重命名")?;
        let pane_id = std::env::var("ZELLIJ_PANE_ID")
            .context("无法获取 ZELLIJ_PANE_ID，跳过 pane 重命名")?;

        let args = vec![
            "--session".to_string(),
            session,
            "action".to_string(),
            "rename-pane".to_string(),
            "--pane-id".to_string(),
            pane_id,
            ctx.name.clone(),
        ];
        crate::notify::run_command("zellij", &args, 2000).await?;
        Ok(())
    }
}

// ── 内置：tmux window 重命名 ───────────────────────────

pub struct TmuxWindowRenamePlugin;

#[async_trait]
impl JoinPlugin for TmuxWindowRenamePlugin {
    fn name(&self) -> &str {
        "tmux-rename-window"
    }

    async fn on_join(&self, ctx: &JoinContext) -> Result<()> {
        let pane = std::env::var("TMUX_PANE")
            .context("当前不在 tmux 中，跳过 window 重命名")?;

        let args = vec![
            "rename-window".to_string(),
            "-t".to_string(),
            pane,
            ctx.name.clone(),
        ];
        crate::notify::run_command("tmux", &args, 2000).await?;
        Ok(())
    }
}
