//! Tauri commands：GUI 前端可调用的 Rust 函数，通过 daemon IPC 获取数据。

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[allow(dead_code)]
async fn daemon_request(req: &str) -> Result<String, String> {
    let path = crate::paths::socket_path();
    let stream = UnixStream::connect(&path).await.map_err(|e| {
        crate::cli::daemon::connection_diagnostic(&path, &format!("{}", e))
    })?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    writer
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())?;

    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    Ok(line)
}

#[allow(dead_code)]
fn request_json(msg: &crate::ipc::ClientMsg) -> String {
    crate::ipc::serialize(msg)
}

#[tauri::command]
#[allow(dead_code)]
pub async fn list_conversations(participant: Option<String>) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::ListConversations { participant };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn list_inbox(
    participant: String,
    status: Option<String>,
    limit: Option<u32>,
    peek: Option<bool>,
) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Inbox {
        sender: Some("gui".into()),
        participant,
        status,
        limit: limit.unwrap_or(50),
        peek: peek.unwrap_or(true),
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_messages(
    conversation_id: String,
    limit: Option<u32>,
    before: Option<String>,
    participant: Option<String>,
) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::GetMessages {
        conversation_id,
        limit: limit.unwrap_or(50),
        before,
        participant,
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_message(
    msg_id: String,
    participant: Option<String>,
) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::GetMessage { msg_id, participant };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn get_attachment(
    attachment_id: String,
    participant: Option<String>,
) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Attachment {
        attachment_id,
        participant,
    };
    daemon_request(&request_json(&msg)).await
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SendPayload {
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[tauri::command]
#[allow(dead_code)]
pub async fn send_message(payload: SendPayload) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Send {
        sender: Some(payload.sender.unwrap_or_else(|| "human".into())),
        to: payload.to,
        body: payload.body,
        conversation_id: payload.conversation_id,
        reply_to: payload.reply_to,
        correlation_id: payload.correlation_id,
        content_type: payload.content_type.unwrap_or_else(|| "text".into()),
        metadata: None,
        notify: false,
        send_enter: None,
        attachments: vec![],
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn mark_done(msg_id: String, participant: String) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Done {
        sender: Some("gui".into()),
        msg_id,
        participant,
        attachments: vec![],
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn mark_read(msg_id: String, participant: String) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Read {
        sender: Some("gui".into()),
        msg_id,
        participant,
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn list_participants(participant_type: Option<String>) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::ListParticipants { participant_type };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn ping_daemon() -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Ping;
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub async fn reply(
    msg_id: String,
    choice: String,
    reason: Option<String>,
    sender: Option<String>,
) -> Result<String, String> {
    let msg = crate::ipc::ClientMsg::Reply {
        sender: Some(sender.unwrap_or_else(|| "human".into())),
        msg_id,
        choice,
        reason: reason.unwrap_or_default(),
    };
    daemon_request(&request_json(&msg)).await
}

#[tauri::command]
#[allow(dead_code)]
pub fn get_popup_focus() -> Option<String> {
    std::env::var("AGTALK_POPUP_MSG_ID").ok()
}
