//! tmux notify 通道：通过 `tmux send-keys` 注入提示。

use super::{build_hint_text, NotifyChannel, NotifyError, NotifyHint, NotifyTarget};

pub struct TmuxChannel;

impl NotifyChannel for TmuxChannel {
    fn name(&self) -> &'static str {
        "tmux"
    }

    fn inject(&self, target: &NotifyTarget, hint: &NotifyHint) -> Result<(), NotifyError> {
        let pane = match target {
            NotifyTarget::Tmux { pane } => pane.as_str(),
            _ => return Err(NotifyError::Other("tmux 通道需要 Tmux target".into())),
        };

        let text = build_hint_text(hint);
        super::run_command("tmux", &["send-keys", "-t", pane, &text, "Enter"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_channel_name() {
        assert_eq!(TmuxChannel.name(), "tmux");
    }
}
