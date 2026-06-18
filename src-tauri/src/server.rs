//! Unix domain socket IPC 服务器：接受 CLI/GUI 连接，处理 ClientMsg，返回 ServerMsg。

use crate::ipc::{self, ClientMsg, ServerMsg};
use crate::storage::Storage;
use crate::transport::TransportRegistry;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

/// Ask 的返回结果
#[derive(Debug, Clone)]
struct AskResult {
    choice: String,
    reason: String,
}

/// 共享的待处理 Ask：msg_id → oneshot sender
type PendingAsks = Arc<Mutex<HashMap<String, oneshot::Sender<AskResult>>>>;

pub async fn run(
    socket_path: &str,
    storage: Arc<Storage>,
    transports: Arc<TransportRegistry>,
) -> Result<()> {
    let _ = std::fs::remove_file(socket_path);
    if let Some(parent) = std::path::Path::new(socket_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    tracing::info!("daemon 监听: {}", socket_path);

    let pending_asks: PendingAsks = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let (stream, _addr) = listener.accept().await?;
        let storage = storage.clone();
        let transports = transports.clone();
        let pending = pending_asks.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, storage, transports, pending).await {
                tracing::error!("连接处理错误: {}", e);
            }
        });
    }
}

async fn handle_connection(
    stream: UnixStream,
    storage: Arc<Storage>,
    transports: Arc<TransportRegistry>,
    pending_asks: PendingAsks,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let msg = match ipc::deserialize::<ClientMsg>(&line) {
            Some(m) => m,
            None => {
                let resp = ServerMsg::Error {
                    code: "parse_error".into(),
                    message: "无法解析消息".into(),
                };
                writer.write_all(ipc::serialize(&resp).as_bytes()).await?;
                continue;
            }
        };

        let response = handle_msg(msg, &storage, &transports, &pending_asks).await;
        writer.write_all(ipc::serialize(&response).as_bytes()).await?;
    }

    Ok(())
}

async fn handle_msg(
    msg: ClientMsg,
    storage: &Storage,
    transports: &TransportRegistry,
    pending_asks: &PendingAsks,
) -> ServerMsg {
    match msg {
        // ── 阻塞式审批请求 ─────────────────────
        ClientMsg::Ask { sender, to, body, choices, timeout_secs } => {
            // 写消息到 DB
            let content_json = serde_json::json!({"choices": &choices, "timeout": timeout_secs});
            let content_json_str = content_json.to_string();
            match storage.send_message(
                &sender, &[to.clone()], &body, "approval_request",
                None, None, None, None, Some(&content_json_str),
            ) {
                Ok(message) => {
                    let msg_id = message.id.clone();

                    // 创建 oneshot 通道
                    let (tx, rx) = oneshot::channel();
                    {
                        let mut pending = pending_asks.lock().unwrap();
                        pending.insert(msg_id.clone(), tx);
                    }

                    // TODO: 通过 transport 通知人类（弹窗/PopupTransport）
                    if let Ok(Some(p)) = storage.get_participant_by_name(&to) {
                        if let Some(t) = transports.get(&p.transport) {
                            let _ = t.deliver(&msg_id, &sender, &body, &p.transport_config).await;
                        }
                    }

                    // 阻塞等待回复或超时
                    let timeout = std::time::Duration::from_secs(timeout_secs);
                    match tokio::time::timeout(timeout, rx).await {
                        Ok(Ok(result)) => {
                            // 写入回复消息：body 存 reason（备注），metadata 存 choice（所选选项）
                            let response_meta =
                                serde_json::json!({"choice": &result.choice}).to_string();
                            let _ = storage.send_message(
                                &to, &[sender.clone()], &result.reason, "approval_response",
                                Some(&msg_id), Some(&message.conversation_id), None, None,
                                Some(&response_meta),
                            );
                            ServerMsg::AskResponse {
                                msg_id,
                                choice: result.choice,
                                reason: result.reason,
                            }
                        }
                        Ok(Err(_)) => {
                            // sender dropped
                            let mut pending = pending_asks.lock().unwrap();
                            pending.remove(&msg_id);
                            ServerMsg::AskTimeout { msg_id }
                        }
                        Err(_) => {
                            // 超时
                            let mut pending = pending_asks.lock().unwrap();
                            pending.remove(&msg_id);
                            ServerMsg::AskTimeout { msg_id }
                        }
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "ask_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        // ── 回复审批请求 ────────────────────
        ClientMsg::Reply { sender: _, msg_id, choice, reason } => {
            let mut pending = pending_asks.lock().unwrap();
            if let Some(tx) = pending.remove(&msg_id) {
                let _ = tx.send(AskResult { choice: choice.clone(), reason: reason.clone() });
                ServerMsg::Ok {
                    data: serde_json::json!({"msg_id": msg_id, "status": "replied"}),
                }
            } else {
                ServerMsg::Error {
                    code: "no_pending_ask".into(),
                    message: format!("没有待处理的 Ask: {}", msg_id),
                }
            }
        }

        // ── 普通消息 ──────────────────────────
        ClientMsg::Send { sender, to, body, conversation_id, reply_to, correlation_id, content_type, metadata } => {
            let metadata = metadata.map(|v| v.to_string());
            match storage.send_message(
                &sender, &[to.clone()], &body, &content_type,
                reply_to.as_deref(), conversation_id.as_deref(),
                correlation_id.as_deref(), None, metadata.as_deref(),
            ) {
                Ok(message) => {
                    if let Ok(Some(participant)) = storage.get_participant_by_name(&to) {
                        if let Some(transport) = transports.get(&participant.transport) {
                            let _ = transport.deliver(&message.id, &sender, &body, &participant.transport_config).await;
                            let _ = storage.mark_delivered(&message.id, &to);
                        }
                    }
                    ServerMsg::Ok {
                        data: serde_json::to_value(&message).unwrap_or_default(),
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "send_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Inbox { sender: _, participant, status: _, limit: _ } => {
            let conversations = storage.list_conversations(Some(&participant)).unwrap_or_default();
            ServerMsg::Ok {
                data: serde_json::to_value(&conversations).unwrap_or_default(),
            }
        }

        ClientMsg::Done { sender: _, msg_id, participant } => {
            match storage.mark_done(&msg_id, &participant) {
                Ok(()) => ServerMsg::Ok { data: serde_json::json!({"msg_id": msg_id}) },
                Err(e) => ServerMsg::Error { code: "done_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::Register { name, participant_type, display_name, transport, transport_config } => {
            let tc = transport_config.to_string();
            match storage.register_participant(&name, &participant_type, &display_name, &transport, &tc) {
                Ok(p) => ServerMsg::Ok {
                    data: serde_json::to_value(&p).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error { code: "register_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::Unregister { name } => {
            match storage.unregister_participant(&name) {
                Ok(()) => ServerMsg::Ok { data: serde_json::json!({"name": name}) },
                Err(e) => ServerMsg::Error { code: "unregister_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::ListParticipants { participant_type } => {
            match storage.list_participants(participant_type.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error { code: "list_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::ListConversations { participant } => {
            match storage.list_conversations(participant.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error { code: "list_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::GetMessages { conversation_id, limit, before } => {
            match storage.get_messages(&conversation_id, limit, before.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error { code: "get_messages_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::Read { sender: _, msg_id, participant } => {
            match storage.mark_read(&msg_id, &participant) {
                Ok(()) => ServerMsg::Ok { data: serde_json::json!({"msg_id": msg_id}) },
                Err(e) => ServerMsg::Error { code: "read_failed".into(), message: e.to_string() },
            }
        }

        ClientMsg::WhoAmI => {
            ServerMsg::Ok {
                data: serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "socket": crate::paths::socket_path(),
                    "participant": std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "me".into()),
                }),
            }
        }

        ClientMsg::CreateConversation { participants, title } => {
            ServerMsg::Ok { data: serde_json::json!({"participants": participants, "title": title, "status": "created"}) }
        }

        ClientMsg::Ping => {
            ServerMsg::Ok { data: serde_json::json!({"pong": true}) }
        }
    }
}
