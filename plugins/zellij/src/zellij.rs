//! zellij 具体操作封装。

use crate::protocol::SendInput;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ZellijError {
    #[error("缺少 zellij session 环境变量")]
    MissingEnv,
    #[error("zellij action 不可用: {0}")]
    ActionUnavailable(String),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("命令执行失败: {0}")]
    CommandFailed(String),
}

/// 从当前 shell 环境发现 zellij endpoint。
pub fn discover() -> Result<(String, String), ZellijError> {
    let session = std::env::var("ZELLIJ_SESSION_NAME").map_err(|_| ZellijError::MissingEnv)?;
    let pane = std::env::var("ZELLIJ_PANE_ID").map_err(|_| ZellijError::MissingEnv)?;

    // 轻量检查：zellij action 是否能访问当前 session。
    let output = Command::new("zellij")
        .args(["--session", &session, "action", "list-panes"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ZellijError::ActionUnavailable(format!(
            "zellij --session {} action list-panes failed: {}",
            session, stderr
        )));
    }

    Ok((session, pane))
}

/// 执行真正的 notify 注入。
pub fn send(input: &SendInput, dry_run: bool) -> Result<(), ZellijError> {
    let session = input.session().ok_or(ZellijError::MissingEnv)?;
    let pane = input.pane().ok_or(ZellijError::MissingEnv)?;

    // 再次检查 action 可用性；dry-run 到此即可。
    let output = Command::new("zellij")
        .args(["--session", &session, "action", "list-panes"])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ZellijError::ActionUnavailable(stderr.to_string()));
    }
    if dry_run {
        return Ok(());
    }

    let text = format!(
        "[agtalk] 新消息来自 {}，运行 {} 查看\n",
        input.from_name, input.read_command
    );

    let output = Command::new("zellij")
        .args([
            "--session",
            &session,
            "action",
            "write-chars",
            "--pane-id",
            &pane,
            &text,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ZellijError::CommandFailed(stderr.to_string()));
    }

    Ok(())
}
