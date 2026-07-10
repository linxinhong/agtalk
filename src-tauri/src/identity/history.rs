//! agent 本地历史：.agtalk/<name>/history.jsonl
//!
//! 定位：agent 回顾对话过程的本地完整日志。不参与认证、路由、SSE、notify。
//! 每次 msg send / reply / ask 后给 sender 和 receiver 追加 message 事件；
//! msg read / done / reply 自动 read/done 时追加 status 事件。

use super::session_file;
use crate::paths::{set_permissions_0600, set_permissions_0700};
use crate::routing::Message;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("路径错误: {0}")]
    Paths(#[from] crate::paths::PathsError),
    #[error("session 不匹配")]
    SessionMismatch,
}

#[derive(Serialize)]
struct HistoryMessageEvent {
    #[serde(rename = "type")]
    event_type: String,
    version: i32,
    recorded_at: f64,
    owner_name: String,
    owner_address: String,
    dir: String,
    id: String,
    short_id: String,
    thread_id: String,
    reply_to: Option<String>,
    from: String,
    from_address: String,
    to: String,
    to_address: String,
    peer: String,
    peer_address: String,
    content_type: String,
    body: String,
    metadata: String,
    event_id: i64,
    status: String,
    created_at: f64,
    source: String,
}

#[derive(Serialize)]
struct HistoryStatusEvent {
    #[serde(rename = "type")]
    event_type: String,
    version: i32,
    recorded_at: f64,
    owner_name: String,
    owner_address: String,
    id: String,
    short_id: String,
    from: String,
    to: String,
    reason: String,
}

fn history_path(dot_agtalk: &Path, name: &str) -> PathBuf {
    dot_agtalk.join(name).join("history.jsonl")
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn short_id(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_string()
}

/// 校验本地 session 存在且 address 匹配。
/// 返回 `true` 表示校验通过；`false` 表示应跳过（session 缺失或 address 不匹配）。
fn validate_owner(dot_agtalk: &Path, name: &str, address: &str) -> bool {
    let session = match session_file::read(dot_agtalk, name) {
        Ok(s) => s,
        Err(_) => return false,
    };
    session.address == address
}

/// 追加 message 事件到 owner 的 history.jsonl。
///
/// - `owner_name` / `owner_address`：历史归属方（sender 或 receiver）。
/// - `dir`：`"out"` 表示 owner 发出的消息；`"in"` 表示 owner 收到的消息。
/// - `source`：触发来源（`msg.send` / `msg.reply` / `msg.ask`）。
pub fn append_message(
    dot_agtalk: &Path,
    owner_name: &str,
    owner_address: &str,
    dir: &str,
    msg: &Message,
    source: &str,
) -> Result<(), HistoryError> {
    if !validate_owner(dot_agtalk, owner_name, owner_address) {
        return Ok(());
    }

    let (peer, peer_address) = if dir == "out" {
        (&msg.to_name, &msg.to_address)
    } else {
        (&msg.from_name, &msg.from_address)
    };

    let thread_id = msg.reply_to_id.clone().unwrap_or_else(|| msg.id.clone());

    let event = HistoryMessageEvent {
        event_type: "message".to_string(),
        version: 1,
        recorded_at: now_secs(),
        owner_name: owner_name.to_string(),
        owner_address: owner_address.to_string(),
        dir: dir.to_string(),
        id: msg.id.clone(),
        short_id: short_id(&msg.id),
        thread_id,
        reply_to: msg.reply_to_id.clone(),
        from: msg.from_name.clone(),
        from_address: msg.from_address.clone(),
        to: msg.to_name.clone(),
        to_address: msg.to_address.clone(),
        peer: peer.clone(),
        peer_address: peer_address.clone(),
        content_type: msg.content_type.clone(),
        body: msg.body.clone(),
        metadata: msg.metadata.clone(),
        event_id: msg.event_id,
        status: msg.status.clone(),
        created_at: msg.created_at,
        source: source.to_string(),
    };

    append_jsonl(dot_agtalk, owner_name, &event)
}

/// 追加 status 事件到 owner 的 history.jsonl。
pub fn append_status(
    dot_agtalk: &Path,
    owner_name: &str,
    owner_address: &str,
    msg_id: &str,
    old_status: &str,
    new_status: &str,
    reason: &str,
) -> Result<(), HistoryError> {
    if !validate_owner(dot_agtalk, owner_name, owner_address) {
        return Ok(());
    }

    let event = HistoryStatusEvent {
        event_type: "status".to_string(),
        version: 1,
        recorded_at: now_secs(),
        owner_name: owner_name.to_string(),
        owner_address: owner_address.to_string(),
        id: msg_id.to_string(),
        short_id: short_id(msg_id),
        from: old_status.to_string(),
        to: new_status.to_string(),
        reason: reason.to_string(),
    };

    append_jsonl(dot_agtalk, owner_name, &event)
}

fn append_jsonl(
    dot_agtalk: &Path,
    owner_name: &str,
    event: &impl Serialize,
) -> Result<(), HistoryError> {
    let dir = dot_agtalk.join(owner_name);
    std::fs::create_dir_all(&dir)?;
    set_permissions_0700(&dir)?;

    let path = history_path(dot_agtalk, owner_name);
    let line = serde_json::to_string(event)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    use std::io::Write;
    writeln!(file, "{}", line)?;
    set_permissions_0600(&path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::session_file::SessionFile;
    use tempfile::TempDir;

    fn dot_with_session(name: &str, address: &str) -> (PathBuf, TempDir) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let session = SessionFile {
            address: address.to_string(),
            name: name.to_string(),
            intro: "test".to_string(),
            ..Default::default()
        };
        session_file::write(&dot, name, &session).unwrap();
        (dot, tmp)
    }

    #[test]
    fn append_message_writes_jsonl() {
        let (dot, _tmp) = dot_with_session("nora", "550e8400-e29b-41d4-a716-446655440000");
        let msg = Message {
            id: "msg-uuid-1234".to_string(),
            to_address: "to".to_string(),
            to_name: "to_name".to_string(),
            from_address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            from_name: "nora".to_string(),
            body: "hello".to_string(),
            content_type: "text".to_string(),
            reply_to_id: None,
            metadata: "{}".to_string(),
            event_id: 7,
            status: "pending".to_string(),
            created_at: 1.0,
        };
        append_message(
            &dot,
            "nora",
            "550e8400-e29b-41d4-a716-446655440000",
            "out",
            &msg,
            "msg.send",
        )
        .unwrap();

        let path = history_path(&dot, "nora");
        let content = std::fs::read_to_string(&path).unwrap();
        let line: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(line["type"], "message");
        assert_eq!(line["dir"], "out");
        assert_eq!(line["source"], "msg.send");
        assert_eq!(line["owner_name"], "nora");
    }

    #[test]
    fn append_status_writes_jsonl() {
        let (dot, _tmp) = dot_with_session("nora", "550e8400-e29b-41d4-a716-446655440000");
        append_status(
            &dot,
            "nora",
            "550e8400-e29b-41d4-a716-446655440000",
            "msg-uuid",
            "pending",
            "read",
            "msg.read",
        )
        .unwrap();

        let path = history_path(&dot, "nora");
        let content = std::fs::read_to_string(&path).unwrap();
        let line: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(line["type"], "status");
        assert_eq!(line["from"], "pending");
        assert_eq!(line["to"], "read");
        assert_eq!(line["reason"], "msg.read");
    }

    #[test]
    fn append_skips_when_session_address_mismatch() {
        let (dot, _tmp) = dot_with_session("nora", "00000000-0000-0000-0000-000000000000");
        let msg = Message {
            id: "msg".to_string(),
            to_address: "to".to_string(),
            to_name: "to".to_string(),
            from_address: "from".to_string(),
            from_name: "nora".to_string(),
            body: "hello".to_string(),
            content_type: "text".to_string(),
            reply_to_id: None,
            metadata: "{}".to_string(),
            event_id: 1,
            status: "pending".to_string(),
            created_at: 1.0,
        };
        // owner_address 与 session 不匹配，应静默跳过
        append_message(
            &dot,
            "nora",
            "550e8400-e29b-41d4-a716-446655440000",
            "out",
            &msg,
            "msg.send",
        )
        .unwrap();
        assert!(!history_path(&dot, "nora").exists());
    }

    #[test]
    fn append_skips_when_session_missing() {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let msg = Message {
            id: "msg".to_string(),
            to_address: "to".to_string(),
            to_name: "to".to_string(),
            from_address: "from".to_string(),
            from_name: "nora".to_string(),
            body: "hello".to_string(),
            content_type: "text".to_string(),
            reply_to_id: None,
            metadata: "{}".to_string(),
            event_id: 1,
            status: "pending".to_string(),
            created_at: 1.0,
        };
        append_message(
            &dot,
            "nora",
            "550e8400-e29b-41d4-a716-446655440000",
            "out",
            &msg,
            "msg.send",
        )
        .unwrap();
        assert!(!history_path(&dot, "nora").exists());
    }
}
