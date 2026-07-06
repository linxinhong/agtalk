//! zellij notify 通道：通过 `zellij action write-chars` 注入提示。

use super::{build_hint_text, NotifyChannel, NotifyError, NotifyHint, NotifyTarget};

pub struct ZellijChannel;

impl NotifyChannel for ZellijChannel {
    fn name(&self) -> &'static str {
        "zellij"
    }

    fn inject(&self, target: &NotifyTarget, hint: &NotifyHint) -> Result<(), NotifyError> {
        let (session, pane) = match target {
            NotifyTarget::Zellij { session, pane } => (session.as_str(), pane.as_str()),
            _ => return Err(NotifyError::Other("zellij 通道需要 Zellij target".into())),
        };

        let text = build_hint_text(hint);
        super::run_command(
            "zellij",
            &[
                "--session",
                session,
                "action",
                "write-chars",
                "--pane-id",
                pane,
                &text,
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zellij_channel_name() {
        assert_eq!(ZellijChannel.name(), "zellij");
    }
}
