//! tmux 具体操作封装。

use crate::protocol::SendInput;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("不在 tmux session 中")]
    MissingEnv,
    #[error("tmux 不可用: {0}")]
    ActionUnavailable(String),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("命令执行失败: {0}")]
    CommandFailed(String),
}

/// 从当前 shell 环境发现 tmux endpoint。
pub fn discover() -> Result<String, TmuxError> {
    if std::env::var("TMUX").is_err() && std::env::var("TMUX_PANE").is_err() {
        return Err(TmuxError::MissingEnv);
    }

    let pane = match std::env::var("TMUX_PANE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            // 尝试询问 tmux 当前 pane id。
            let output = Command::new("tmux")
                .args(["display-message", "-p", "#D"])
                .output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(TmuxError::ActionUnavailable(stderr.to_string()));
            }
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    if pane.is_empty() {
        return Err(TmuxError::MissingEnv);
    }

    // 轻量检查 pane 是否可达。
    let output = Command::new("tmux")
        .args(["list-panes", "-t", &pane])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::ActionUnavailable(stderr.to_string()));
    }

    Ok(pane)
}

/// 执行真正的 notify 注入。
pub fn send(input: &SendInput, dry_run: bool) -> Result<(), TmuxError> {
    let pane = input.pane().ok_or(TmuxError::MissingEnv)?;

    let output = Command::new("tmux")
        .args(["list-panes", "-t", &pane])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::ActionUnavailable(stderr.to_string()));
    }
    if dry_run {
        return Ok(());
    }

    let text = format!(
        "[agtalk] 新消息来自 {}，运行 {} 查看\n",
        input.from_name, input.read_command
    );

    let output = Command::new("tmux")
        .args(["send-keys", "-t", &pane, "-R", &text, "Enter"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed(stderr.to_string()));
    }

    Ok(())
}
