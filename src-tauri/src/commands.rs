//! Tauri 命令桥（薄）。
//! 每个 #[tauri::command] 只做：接收前端参数 → 调对应领域函数 → 返回结果。
//! popup 窗口的业务全部经 HumanClient（human/client.rs）走 daemon API，
//! token 留在 Rust 侧，不下发到前端。

use crate::human::client::HumanClient;
use crate::human::popup::POPUP_SURFACE;
use crate::routing::Message;
use serde::Serialize;

/// popup 窗口状态：本窗口对应的消息 id（由 `agtalk __popup <msg-id>` 传入）。
pub struct PopupState {
    pub message_id: String,
}

/// popup 展示模型：消息详情 + human 地址（前端展示用）。
#[derive(Debug, Serialize)]
pub struct PopupView {
    pub message: Message,
    pub human_address: String,
}

/// 加载消息：标 read 取详情，并回执 delivery（ack 失败只记录，不影响展示）。
pub(crate) fn load_view(message_id: &str) -> Result<PopupView, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    let human_address = client.address().to_string();
    let message = client.read(message_id).map_err(|e| e.to_string())?;
    if let Err(e) = client.delivery_ack(POPUP_SURFACE, message_id) {
        tracing::warn!("popup delivery ack failed: {}", e);
    }
    Ok(PopupView {
        message,
        human_address,
    })
}

/// 回复消息；choice 用于审批选项。返回回复消息 id。
pub(crate) fn reply_message(
    message_id: &str,
    body: &str,
    choice: Option<&str>,
) -> Result<String, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .reply(POPUP_SURFACE, message_id, body, choice)
        .map_err(|e| e.to_string())
}

/// 标记消息完成。
pub(crate) fn done_message(message_id: &str) -> Result<(), String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .done(POPUP_SURFACE, message_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn popup_load(state: tauri::State<'_, PopupState>) -> Result<PopupView, String> {
    load_view(&state.message_id)
}

#[tauri::command]
pub fn popup_reply(
    state: tauri::State<'_, PopupState>,
    body: String,
    choice: Option<String>,
) -> Result<String, String> {
    reply_message(&state.message_id, &body, choice.as_deref())
}

#[tauri::command]
pub fn popup_done(state: tauri::State<'_, PopupState>) -> Result<(), String> {
    done_message(&state.message_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

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

    #[test]
    fn commands_without_human_session_return_stable_error() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        // 无 session.json：三个命令都应返回 session 错误而不是 panic
        let load_err = load_view("any-id").unwrap_err();
        assert!(load_err.contains("session"), "load: {}", load_err);
        let reply_err = reply_message("any-id", "hi", None).unwrap_err();
        assert!(reply_err.contains("session"), "reply: {}", reply_err);
        let done_err = done_message("any-id").unwrap_err();
        assert!(done_err.contains("session"), "done: {}", done_err);
    }
}
