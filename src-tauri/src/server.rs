//! Unix domain socket IPC 服务器：接受 CLI/GUI 连接，处理 ClientMsg，返回 ServerMsg。

use crate::ipc::{self, ClientMsg, ServerMsg};
use crate::notify::{self, NotifyContext, NotifyPluginRegistry, NotifyTransportConfig};
use crate::storage::{endpoint_key_from_notify_config, SessionInfo, Storage};
use crate::transport::TransportRegistry;
use anyhow::Result;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

/// Ask 的返回结果
#[derive(Debug, Clone)]
pub(crate) struct AskResult {
    choice: String,
    reason: String,
}

/// 一个 waiter 的注册句柄，用于超时后安全地只清理自己
pub(crate) struct Waiter {
    id: String,
    tx: oneshot::Sender<AskResult>,
}

/// 共享的待处理 Ask：msg_id → 所有在等待的 waiter
pub(crate) type PendingAsks = Arc<Mutex<HashMap<String, Vec<Waiter>>>>;

/// PollInbox 的等待键
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PollKey {
    pub workspace_id: String,
    pub participant_id: String,
}

/// 共享的 PollInbox waiter：每个 participant 同时只允许一个挂起 poll。
pub(crate) type PollWaiters = Arc<Mutex<HashMap<PollKey, oneshot::Sender<()>>>>;

/// 确保 PollInbox waiter 在 future 被 cancel / 正常返回 / panic 时都能被清理。
struct PollWaiterGuard {
    waiters: PollWaiters,
    key: PollKey,
}

impl Drop for PollWaiterGuard {
    fn drop(&mut self) {
        self.waiters.lock().unwrap().remove(&self.key);
    }
}

/// 将本地 .agtalk/sessions/<name>.json 的状态标记为 left。
fn mark_local_session_left(name: &str) -> Result<()> {
    if let Some(mut sf) = crate::session::read_session(name)? {
        sf.session.status = "left".into();
        crate::session::write_session(name, &mut sf)?;
    }
    Ok(())
}

pub(crate) struct HttpState {
    pub storage: Arc<Storage>,
    pub transports: Arc<TransportRegistry>,
    pub notify_plugins: Arc<NotifyPluginRegistry>,
    pub pending_asks: PendingAsks,
    pub poll_waiters: PollWaiters,
}

pub async fn run(
    socket_path: &str,
    http_port: u16,
    storage: Arc<Storage>,
    transports: Arc<TransportRegistry>,
    notify_plugins: Arc<NotifyPluginRegistry>,
) -> Result<()> {
    let _ = std::fs::remove_file(socket_path);
    if let Some(parent) = std::path::Path::new(socket_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    tracing::info!("daemon 监听: {}", socket_path);

    let http_addr = SocketAddr::from(([127, 0, 0, 1], http_port));
    let http_listener = tokio::net::TcpListener::bind(http_addr).await?;
    tracing::info!("daemon HTTP 监听: 127.0.0.1:{}", http_port);

    let pending_asks: PendingAsks = Arc::new(Mutex::new(HashMap::new()));
    let poll_waiters: PollWaiters = Arc::new(Mutex::new(HashMap::new()));

    let state = Arc::new(HttpState {
        storage: storage.clone(),
        transports: transports.clone(),
        notify_plugins: notify_plugins.clone(),
        pending_asks: pending_asks.clone(),
        poll_waiters: poll_waiters.clone(),
    });

    let app = Router::new()
        .route("/api", post(handle_http))
        .with_state(state);

    // HTTP 服务在后台运行，Unix socket 保持主 accept 循环
    let http_server = axum::serve(http_listener, app);
    tokio::spawn(async move {
        if let Err(e) = http_server.await {
            tracing::error!("HTTP 服务异常: {}", e);
        }
    });

    loop {
        let (stream, _addr) = listener.accept().await?;
        let storage = storage.clone();
        let transports = transports.clone();
        let notify_plugins = notify_plugins.clone();
        let pending = pending_asks.clone();
        let waiters = poll_waiters.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_connection(stream, storage, transports, notify_plugins, pending, waiters).await
            {
                tracing::error!("连接处理错误: {}", e);
            }
        });
    }
}

/// HTTP 入口：解析 ClientMsg，按需从 header 认证，调用 handle_msg 后返回 ServerMsg。
pub(crate) async fn handle_http(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Json(msg): Json<ClientMsg>,
) -> (StatusCode, Json<ServerMsg>) {
    let mut session: Option<SessionInfo> = None;

    // 免认证操作：Ping、Auth、Join、Attach、ListParticipants（与 CLI peers 一致）
    let needs_auth = !matches!(
        msg,
        ClientMsg::Ping
            | ClientMsg::Auth { .. }
            | ClientMsg::Join { .. }
            | ClientMsg::Attach { .. }
            | ClientMsg::ListParticipants { .. }
    );

    if needs_auth {
        // 1) 优先使用 X-Agtalk-Session-Id + X-Agtalk-Token
        let session_id = headers
            .get("X-Agtalk-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let token = headers
            .get("X-Agtalk-Token")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        if let (Some(sid), Some(tok)) = (session_id, token) {
            match state.storage.validate_session(&sid, &tok) {
                Ok(Some(info)) => {
                    session = Some(info);
                }
                Ok(None) => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ServerMsg::Error {
                            code: "auth_failed".into(),
                            message: "session_id 或 token 无效".into(),
                        }),
                    );
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ServerMsg::Error {
                            code: "auth_error".into(),
                            message: format!("校验 session 失败: {}", e),
                        }),
                    );
                }
            }
        }

        // 2) PollInbox 等敏感接口不允许 name fallback，必须显式提供 session_id/token
        let requires_strict_auth = matches!(msg, ClientMsg::PollInbox { .. });
        if session.is_none() && requires_strict_auth {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ServerMsg::Error {
                    code: "poll_inbox_requires_session_token".into(),
                    message: "PollInbox 必须提供 X-Agtalk-Session-Id 和 X-Agtalk-Token".into(),
                }),
            );
        }

        // 3) 未传 session_id/token 时，fallback 到 X-Agtalk-Name：取该 participant 的 active session
        if session.is_none() {
            if let Some(name) = headers.get("X-Agtalk-Name").and_then(|v| v.to_str().ok()) {
                if let Ok(Some(p)) = state.storage.get_participant_by_name(name) {
                    if let Ok(Some((sid, tok))) =
                        state.storage.get_active_session_id_and_token(&p.id)
                    {
                        if let Ok(Some(info)) = state.storage.validate_session(&sid, &tok) {
                            session = Some(info);
                        }
                    }
                }
            }
        }

        // 4) 仍然无 session 则拒绝
        if session.is_none() {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ServerMsg::Error {
                    code: "auth_required".into(),
                    message: "缺少 X-Agtalk-Session-Id / X-Agtalk-Token 或 X-Agtalk-Name header".into(),
                }),
            );
        }
    }

    let response = handle_msg(
        msg,
        &state.storage,
        &state.transports,
        &state.notify_plugins,
        &state.pending_asks,
        &state.poll_waiters,
        &mut session,
    )
    .await;

    (StatusCode::OK, Json(response))
}

async fn handle_connection(
    stream: UnixStream,
    storage: Arc<Storage>,
    transports: Arc<TransportRegistry>,
    notify_plugins: Arc<NotifyPluginRegistry>,
    pending_asks: PendingAsks,
    poll_waiters: PollWaiters,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut session: Option<SessionInfo> = None;

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

        let response = handle_msg(
            msg,
            &storage,
            &transports,
            &notify_plugins,
            &pending_asks,
            &poll_waiters,
            &mut session,
        )
        .await;
        writer
            .write_all(ipc::serialize(&response).as_bytes())
            .await?;
    }

    Ok(())
}

fn sender_from(session: &Option<SessionInfo>, explicit: Option<String>) -> String {
    // 如果客户端显式传了 sender（旧兼容），先用显式的；否则用 session 身份
    if let Some(s) = explicit.filter(|s| !s.is_empty()) {
        return s;
    }
    session
        .as_ref()
        .map(|s| s.participant_name.clone())
        .unwrap_or_else(|| "human".into())
}

fn session_id_from(session: &Option<SessionInfo>) -> Option<String> {
    session.as_ref().map(|s| s.session_id.clone())
}

/// 优先使用客户端显式指定的 viewer，否则回退到 session 身份。
/// GUI / popup 进程没有 session，人类相关查看默认以 "human" 身份进行。
fn viewer_from(session: &Option<SessionInfo>, explicit: Option<String>) -> String {
    explicit
        .filter(|s| !s.is_empty())
        .or_else(|| session.as_ref().map(|s| s.participant_name.clone()))
        .unwrap_or_else(|| "human".into())
}

/// 从 storage 读取已持久化的 approval_response，返回 (choice, reason)。
fn read_approval_response(storage: &Storage, msg_id: &str) -> Option<(String, String)> {
    let msg = storage.get_approval_response(msg_id).ok().flatten()?;
    let choice = serde_json::from_str::<serde_json::Value>(&msg.metadata)
        .ok()
        .and_then(|v| v.get("choice").and_then(|c| c.as_str()).map(String::from))
        .unwrap_or_default();
    Some((choice, msg.body))
}

/// 注册一个 waiter，返回 (waiter_id, receiver)。
fn register_waiter(pending_asks: &PendingAsks, msg_id: &str) -> (String, oneshot::Receiver<AskResult>) {
    let (tx, rx) = oneshot::channel();
    let id = uuid::Uuid::new_v4().to_string();
    pending_asks
        .lock()
        .unwrap()
        .entry(msg_id.to_string())
        .or_default()
        .push(Waiter { id: id.clone(), tx });
    (id, rx)
}

/// 只移除指定 waiter_id 的注册，避免误删其他 waiter。
fn remove_waiter(pending_asks: &PendingAsks, msg_id: &str, waiter_id: &str) {
    let mut pending = pending_asks.lock().unwrap();
    if let Some(vec) = pending.get_mut(msg_id) {
        vec.retain(|w| w.id != waiter_id);
        if vec.is_empty() {
            pending.remove(msg_id);
        }
    }
}

/// 通知并清空指定 msg_id 的所有 waiter。
fn notify_waiters(pending_asks: &PendingAsks, msg_id: &str, result: AskResult) {
    let waiters = {
        let mut pending = pending_asks.lock().unwrap();
        pending.remove(msg_id)
    };
    if let Some(vec) = waiters {
        for waiter in vec {
            let _ = waiter.tx.send(result.clone());
        }
    }
}

/// 等待一个已注册的 receiver，超时后检查 storage 兜底。
async fn await_reply(
    rx: oneshot::Receiver<AskResult>,
    storage: &Storage,
    msg_id: &str,
    timeout: std::time::Duration,
) -> Option<(String, String)> {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => Some((result.choice, result.reason)),
        _ => {
            // 超时或被取消后，再查一次 storage，处理注册 waiter 与 Reply 的竞态
            read_approval_response(storage, msg_id)
        }
    }
}

/// 执行一次 notify（按 session 级别 notify_config）
async fn try_notify(
    storage: &Storage,
    notify_plugins: &NotifyPluginRegistry,
    from: &str,
    to: &str,
    body: &str,
    msg_id: &str,
    send_enter: Option<bool>,
) -> Result<(), String> {
    let participant = storage
        .get_participant_by_name(to)
        .map_err(|e| format!("查询接收者失败: {}", e))?
        .ok_or_else(|| format!("接收者不存在: {}", to))?;
    let session = storage
        .get_active_session_for_participant(&participant.id)
        .map_err(|e| format!("查询 active session 失败: {}", e))?
        .ok_or_else(|| format!("目标 {} 没有 active session", to))?;
    let notify_cfg_str = session
        .notify_config
        .as_deref()
        .ok_or_else(|| format!("目标 {} 的 session 没有 notify_config", to))?;
    let notify_cfg: NotifyTransportConfig = serde_json::from_str(notify_cfg_str)
        .map_err(|e| format!("notify_config 格式错误: {}", e))?;
    let plugin = notify_plugins
        .get(&notify_cfg.plugin)
        .ok_or_else(|| format!("notify 插件未加载: {}", notify_cfg.plugin))?;
    let effective_send_enter = send_enter.unwrap_or(notify_cfg.send_enter);
    let ctx = NotifyContext {
        message_id: msg_id.to_string(),
        short_message_id: notify::short_id(msg_id),
        from: from.to_string(),
        to: to.to_string(),
        text: body.to_string(),
        command: "agtalk detail -".to_string(),
        send_enter: effective_send_enter,
        endpoint: notify_cfg.endpoint,
    };
    plugin.notify(&ctx).await.map_err(|e| e.to_string())
}

/// 执行一次 inbox 查询，并尝试将返回的 pending 消息标记为 delivered。
fn poll_inbox_once(
    storage: &Storage,
    participant_name: &str,
    filter: &str,
    limit: u32,
) -> Result<(Vec<crate::storage::InboxItem>, Vec<String>)> {
    let items = storage.list_inbox(participant_name, Some(filter), limit)?;
    let pending_ids: Vec<String> = items
        .iter()
        .filter(|i| i.delivery.status == "pending")
        .map(|i| i.id.clone())
        .collect();
    if !pending_ids.is_empty() {
        storage.mark_delivered_for_messages(&pending_ids, participant_name)?;
    }
    Ok((items, pending_ids))
}

/// 通知指定 participant 的挂起 PollInbox。
fn notify_poll_waiter(
    storage: &Storage,
    poll_waiters: &PollWaiters,
    participant_id: &str,
) {
    let workspace_id = storage
        .get_active_session_for_participant(participant_id)
        .ok()
        .flatten()
        .map(|s| s.workspace_id)
        .unwrap_or_default();
    let key = PollKey {
        workspace_id,
        participant_id: participant_id.to_string(),
    };
    let tx = poll_waiters.lock().unwrap().remove(&key);
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
}

pub(crate) async fn handle_msg(
    msg: ClientMsg,
    storage: &Storage,
    transports: &TransportRegistry,
    notify_plugins: &NotifyPluginRegistry,
    pending_asks: &PendingAsks,
    poll_waiters: &PollWaiters,
    session: &mut Option<SessionInfo>,
) -> ServerMsg {
    // 已认证连接：每次请求重新校验 session 仍 active，并刷新活跃时间
    if let Some(ref s) = session {
        match storage.is_session_active(&s.session_id) {
            Ok(true) => {
                let _ = storage.touch_session(&s.session_id);
            }
            Ok(false) => {
                *session = None;
                return ServerMsg::Error {
                    code: "session_inactive".into(),
                    message: "当前 session 已被退役或接管".into(),
                };
            }
            Err(e) => {
                return ServerMsg::Error {
                    code: "session_error".into(),
                    message: format!("校验 session 失败: {}", e),
                };
            }
        }
    }

    match msg {
        // ── 认证 ──────────────────────────────
        ClientMsg::Auth { session_id, token } => {
            match storage.validate_session(&session_id, &token) {
                Ok(Some(info)) => {
                    *session = Some(info.clone());
                    let _ = storage.touch_session(&session_id);
                    let _ = storage.update_participant_status(&info.participant_name, "online");
                    ServerMsg::Ok {
                        data: serde_json::to_value(&info).unwrap_or_default(),
                    }
                }
                Ok(None) => ServerMsg::Error {
                    code: "auth_failed".into(),
                    message: "session_id 或 token 无效".into(),
                },
                Err(e) => ServerMsg::Error {
                    code: "auth_error".into(),
                    message: e.to_string(),
                },
            }
        }

        // ── 加入 workspace / 创建 session ─────
        ClientMsg::Join {
            workspace_root,
            workspace_name,
            name,
            participant_type,
            role,
            intro,
            transport,
            notify_config,
            runtime_config,
            capabilities: _,
            takeover,
        } => {
            let participant_type = participant_type.unwrap_or_else(|| "agent".into());
            const RESERVED_NAMES: &[&str] = &["me", "human"];
            if RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
                return ServerMsg::Error {
                    code: "reserved_name".into(),
                    message: format!("'{}' 是保留名称，不能注册为 participant", name),
                };
            }

            let workspace_id = match storage.register_workspace(&workspace_name, &workspace_root) {
                Ok(id) => id,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "join_failed".into(),
                        message: format!("注册 workspace 失败: {}", e),
                    }
                }
            };
            let notify_config_str = if notify_config.is_null() {
                None
            } else {
                Some(notify_config.to_string())
            };
            let runtime_config_str = if runtime_config.is_null() {
                None
            } else {
                Some(runtime_config.to_string())
            };
            let endpoint_key = endpoint_key_from_notify_config(&notify_config);
            let has_endpoint = !endpoint_key.is_empty();

            // 无 endpoint 的 shell join 不参与冲突检测；有 endpoint 且未 takeover 时检查冲突
            if has_endpoint && !takeover {
                match storage.get_active_sessions_by_endpoint(&workspace_id, &endpoint_key) {
                    Ok(existing) if !existing.is_empty() => {
                        return ServerMsg::Error {
                            code: "session_conflict".into(),
                            message: format!(
                                "endpoint {} 上已有 {} 个 active session；如要接管请重试并设置 takeover=true",
                                endpoint_key,
                                existing.len()
                            ),
                        };
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: format!("查询 endpoint 冲突失败: {}", e),
                        };
                    }
                }
            }

            let participant = match storage.get_participant_by_name(&name) {
                Ok(Some(p)) => {
                    // 已存在则更新 type/role/intro/transport/status
                    let _ = storage.update_participant_on_join(&name, &participant_type, &role, &intro, &transport);
                    p
                }
                Ok(None) => match storage.register_participant(
                    Some(&name),
                    &name,
                    &participant_type,
                    &name,
                    &transport,
                    "{}",
                    &intro,
                    &role,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: format!("注册 participant 失败: {}", e),
                        }
                    }
                },
                Err(e) => {
                    return ServerMsg::Error {
                        code: "join_failed".into(),
                        message: format!("查询 participant 失败: {}", e),
                    }
                }
            };
            // 有 endpoint 且 takeover 时：原子地创建 session 并退役同 endpoint 旧 session。
            // create_session_with_takeover 内部使用事务，任一步失败都会回滚，旧 session 保持 active。
            let (session_id, token, retired) = if has_endpoint && takeover {
                match storage.create_session_with_takeover(
                    &workspace_id,
                    &participant.id,
                    &endpoint_key,
                    notify_config_str.as_deref(),
                    runtime_config_str.as_deref(),
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: format!("创建 session 失败: {}", e),
                        }
                    }
                }
            } else {
                match storage.create_session(
                    &workspace_id,
                    &participant.id,
                    &endpoint_key,
                    notify_config_str.as_deref(),
                    runtime_config_str.as_deref(),
                ) {
                    Ok((sid, tok)) => (sid, tok, vec![]),
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: format!("创建 session 失败: {}", e),
                        }
                    }
                }
            };

            // 同步被接管旧 session 的本地文件并重新计算 participant 在线状态
            for old in retired {
                let _ = mark_local_session_left(&old.participant_name);
                let _ = storage.recompute_participant_status(&old.participant_id);
            }

            // 自动认证当前连接为刚创建的 session
            if let Ok(Some(info)) = storage.validate_session(&session_id, &token) {
                *session = Some(info.clone());
            }

            let _ = storage.update_participant_status(&name, "online");

            ServerMsg::Ok {
                data: serde_json::json!({
                    "workspace_id": workspace_id,
                    "participant_id": participant.id,
                    "session_id": session_id,
                    "token": token,
                }),
            }
        }

        // ── 接管已有 peer 身份 ────────────────
        ClientMsg::Attach {
            workspace_root,
            workspace_name,
            name,
            notify_config,
            runtime_config,
            takeover,
        } => {
            let workspace_id = match storage.register_workspace(&workspace_name, &workspace_root) {
                Ok(id) => id,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "attach_failed".into(),
                        message: format!("注册 workspace 失败: {}", e),
                    }
                }
            };
            let notify_config_str = if notify_config.is_null() {
                None
            } else {
                Some(notify_config.to_string())
            };
            let runtime_config_str = if runtime_config.is_null() {
                None
            } else {
                Some(runtime_config.to_string())
            };
            let endpoint_key = endpoint_key_from_notify_config(&notify_config);
            let has_endpoint = !endpoint_key.is_empty();

            // 无 endpoint 的 shell attach 不参与冲突检测；有 endpoint 且未 takeover 时检查冲突
            if has_endpoint && !takeover {
                match storage.get_active_sessions_by_endpoint(&workspace_id, &endpoint_key) {
                    Ok(existing) if !existing.is_empty() => {
                        return ServerMsg::Error {
                            code: "session_conflict".into(),
                            message: format!(
                                "endpoint {} 上已有 {} 个 active session；如要接管请重试并设置 takeover=true",
                                endpoint_key,
                                existing.len()
                            ),
                        };
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "attach_failed".into(),
                            message: format!("查询 endpoint 冲突失败: {}", e),
                        };
                    }
                }
            }

            // attach 要求 peer 必须已存在；不修改 role/intro/transport
            let participant = match storage.get_participant_by_name(&name) {
                Ok(Some(p)) => p,
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "participant_not_found".into(),
                        message: format!("participant '{}' 不存在，请先用 join 注册", name),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "attach_failed".into(),
                        message: format!("查询 participant 失败: {}", e),
                    }
                }
            };

            let (session_id, token, retired) = if has_endpoint && takeover {
                match storage.create_session_with_takeover(
                    &workspace_id,
                    &participant.id,
                    &endpoint_key,
                    notify_config_str.as_deref(),
                    runtime_config_str.as_deref(),
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "attach_failed".into(),
                            message: format!("创建 session 失败: {}", e),
                        }
                    }
                }
            } else {
                match storage.create_session(
                    &workspace_id,
                    &participant.id,
                    &endpoint_key,
                    notify_config_str.as_deref(),
                    runtime_config_str.as_deref(),
                ) {
                    Ok((sid, tok)) => (sid, tok, vec![]),
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "attach_failed".into(),
                            message: format!("创建 session 失败: {}", e),
                        }
                    }
                }
            };

            // 同步被接管旧 session 的本地文件并重新计算 participant 在线状态
            for old in retired {
                let _ = mark_local_session_left(&old.participant_name);
                let _ = storage.recompute_participant_status(&old.participant_id);
            }

            // 自动认证当前连接为刚创建的 session
            if let Ok(Some(info)) = storage.validate_session(&session_id, &token) {
                *session = Some(info.clone());
            }

            let _ = storage.update_participant_status(&name, "online");

            ServerMsg::Ok {
                data: serde_json::json!({
                    "workspace_id": workspace_id,
                    "participant_id": participant.id,
                    "session_id": session_id,
                    "token": token,
                }),
            }
        }

        // ── 离开 session ──────────────────────
        ClientMsg::Leave { session_id } => {
            let sid = session_id.or_else(|| session.as_ref().map(|s| s.session_id.clone()));
            match sid {
                Some(sid) => match storage.get_session_by_id(&sid) {
                    Ok(Some(info)) => match storage.mark_session_left(&sid) {
                        Ok(()) => {
                            if session
                                .as_ref()
                                .map(|s| s.session_id == sid)
                                .unwrap_or(false)
                            {
                                *session = None;
                            }
                            let _ = mark_local_session_left(&info.participant_name);
                            let _ = storage.recompute_participant_status(&info.participant_id);
                            ServerMsg::Ok {
                                data: serde_json::json!({"session_id": sid, "status": "left"}),
                            }
                        }
                        Err(e) => ServerMsg::Error {
                            code: "leave_failed".into(),
                            message: e.to_string(),
                        },
                    },
                    Ok(None) => ServerMsg::Error {
                        code: "leave_failed".into(),
                        message: format!("session {} 不存在", sid),
                    },
                    Err(e) => ServerMsg::Error {
                        code: "leave_failed".into(),
                        message: e.to_string(),
                    },
                },
                None => ServerMsg::Error {
                    code: "leave_failed".into(),
                    message: "未提供 session_id 且当前未认证".into(),
                },
            }
        }

        // ── 清理 inactive session ─────────────
        ClientMsg::Cleanup { workspace_id, dry_run } => {
            match storage.cleanup_inactive_sessions(&workspace_id, dry_run) {
                Ok(names) => ServerMsg::Ok {
                    data: serde_json::json!({
                        "dry_run": dry_run,
                        "removed": names,
                    }),
                },
                Err(e) => ServerMsg::Error {
                    code: "cleanup_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        // ── 阻塞式审批请求 ─────────────────────
        ClientMsg::Ask {
            sender,
            to,
            body,
            choices,
            timeout_secs,
        } => {
            let sender = sender_from(session, sender);
            let content_json = serde_json::json!({"choices": &choices, "timeout": timeout_secs});
            let content_json_str = content_json.to_string();
            match storage.send_message(
                &sender,
                std::slice::from_ref(&to),
                &body,
                "approval_request",
                None,
                None,
                None,
                None,
                Some(&content_json_str),
            ) {
                Ok(message) => {
                    let msg_id = message.id.clone();

                    if let Ok(Some(p)) = storage.get_participant_by_name(&to) {
                        notify_poll_waiter(storage, poll_waiters, &p.id);
                    }

                    // 1. 若回复已持久化（极速回复），直接返回
                    if let Some((choice, reason)) = read_approval_response(storage, &msg_id) {
                        return ServerMsg::AskResponse {
                            msg_id,
                            choice,
                            reason,
                        };
                    }

                    // 2. 先注册 waiter，再 deliver，避免“deliver 后、注册前”收到 Reply 导致的竞态
                    let (waiter_id, rx) = register_waiter(pending_asks, &msg_id);

                    if let Ok(Some(p)) = storage.get_participant_by_name(&to) {
                        if let Some(t) = transports.get(&p.transport) {
                            match t.deliver(&msg_id, &sender, &body, &p.transport_config).await {
                                Ok(Some(monitor)) => {
                                    // 监控子进程：若弹窗被关闭且尚未收到回复，
                                    // 立即通知所有 waiter 视为 dismissed。
                                    let pending = pending_asks.clone();
                                    let msg_id2 = msg_id.clone();
                                    tokio::spawn(async move {
                                        match monitor.wait().await {
                                            Ok(_status) => {
                                                {
                                                    let map = pending.lock().unwrap();
                                                    if !map.contains_key(&msg_id2) {
                                                        return;
                                                    }
                                                }
                                                notify_waiters(
                                                    &pending,
                                                    &msg_id2,
                                                    AskResult {
                                                        choice: "__dismissed__".into(),
                                                        reason: "人类关闭了弹窗，未作出选择".into(),
                                                    },
                                                );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "监控弹窗子进程失败 msg={}: {}",
                                                    msg_id2,
                                                    e
                                                );
                                            }
                                        }
                                    });
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::error!(
                                        "deliver 失败 msg={} transport={}: {}",
                                        msg_id,
                                        p.transport,
                                        e
                                    );
                                }
                            }
                        }
                    }

                    let timeout = std::time::Duration::from_secs(timeout_secs);
                    let result = await_reply(rx, storage, &msg_id, timeout).await;
                    remove_waiter(pending_asks, &msg_id, &waiter_id);

                    match result {
                        Some((choice, reason)) => {
                            if choice == "__dismissed__" {
                                ServerMsg::AskDismissed { msg_id }
                            } else {
                                ServerMsg::AskResponse {
                                    msg_id,
                                    choice,
                                    reason,
                                }
                            }
                        }
                        None => ServerMsg::AskTimeout { msg_id },
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "ask_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        // ── 回复审批请求 ────────────────────
        ClientMsg::Reply {
            sender,
            msg_id,
            choice,
            reason,
        } => {
            let sender = sender_from(session, sender);
            let original = match storage.get_message_by_id(
                &msg_id,
                Some(&sender),
                session_id_from(session).as_deref(),
            ) {
                Ok(Some(m)) if m.content_type == "approval_request" => m,
                Ok(Some(_)) => {
                    return ServerMsg::Error {
                        code: "not_approval_request".into(),
                        message: format!("消息 {} 不是审批请求", msg_id),
                    }
                }
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "message_not_found".into(),
                        message: format!("消息不存在: {}", msg_id),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "lookup_failed".into(),
                        message: e.to_string(),
                    }
                }
            };

            // 先持久化 approval_response，再通知内存中的 waiter
            let response_meta = serde_json::json!({"choice": &choice}).to_string();
            if let Err(e) = storage.send_message(
                &sender,
                std::slice::from_ref(&original.sender_name),
                &reason,
                "approval_response",
                Some(&msg_id),
                Some(&original.conversation_id),
                None,
                None,
                Some(&response_meta),
            ) {
                return ServerMsg::Error {
                    code: "reply_failed".into(),
                    message: e.to_string(),
                };
            }

            if let Ok(Some(participant)) = storage.get_participant_by_name(&original.sender_name) {
                notify_poll_waiter(storage, poll_waiters, &participant.id);
            }

            notify_waiters(
                pending_asks,
                &msg_id,
                AskResult {
                    choice: choice.clone(),
                    reason: reason.clone(),
                },
            );

            ServerMsg::Ok {
                data: serde_json::json!({"msg_id": msg_id, "status": "replied"}),
            }
        }

        // ── 等待审批结果（长轮询）───────────────────
        ClientMsg::Wait {
            sender,
            msg_id,
            timeout_secs,
        } => {
            let sender = sender_from(session, sender);
            let original = match storage.get_message_by_id(
                &msg_id,
                Some(&sender),
                session_id_from(session).as_deref(),
            ) {
                Ok(Some(m)) if m.content_type == "approval_request" => m,
                Ok(Some(_)) => {
                    return ServerMsg::Error {
                        code: "not_approval_request".into(),
                        message: format!("消息 {} 不是审批请求", msg_id),
                    }
                }
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "message_not_found".into(),
                        message: format!("消息不存在: {}", msg_id),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "lookup_failed".into(),
                        message: e.to_string(),
                    }
                }
            };

            if original.sender_name != sender {
                return ServerMsg::Error {
                    code: "unauthorized_wait".into(),
                    message: format!("只有 Ask 的发起者可以等待其审批结果: {}", msg_id),
                };
            }

            // 先检查是否已持久化回复
            if let Some((choice, reason)) = read_approval_response(storage, &msg_id) {
                return ServerMsg::WaitResult {
                    msg_id,
                    status: "replied".into(),
                    choice,
                    reason,
                    timed_out: false,
                };
            }

            let timeout = std::time::Duration::from_secs(timeout_secs);
            let (waiter_id, rx) = register_waiter(pending_asks, &msg_id);
            let result = await_reply(rx, storage, &msg_id, timeout).await;
            remove_waiter(pending_asks, &msg_id, &waiter_id);

            // 最后再查一次 storage，处理 timeout 临界时 Reply 已写库但未通知到本 waiter 的情况
            if result.is_none() {
                if let Some((choice, reason)) = read_approval_response(storage, &msg_id) {
                    return ServerMsg::WaitResult {
                        msg_id: msg_id.clone(),
                        status: "replied".into(),
                        choice,
                        reason,
                        timed_out: false,
                    };
                }
            }

            match result {
                Some((choice, reason)) => ServerMsg::WaitResult {
                    msg_id,
                    status: "replied".into(),
                    choice,
                    reason,
                    timed_out: false,
                },
                None => ServerMsg::WaitResult {
                    msg_id,
                    status: "timed_out".into(),
                    choice: String::new(),
                    reason: String::new(),
                    timed_out: true,
                },
            }
        }

        // ── 普通消息 ──────────────────────────
        ClientMsg::Send {
            sender,
            to,
            body,
            conversation_id,
            reply_to,
            correlation_id,
            content_type,
            metadata,
            notify,
            send_enter,
            attachments,
        } => {
            let sender = sender_from(session, sender);
            let metadata = metadata.map(|v| v.to_string());
            match storage.send_message_with_attachments(
                &sender,
                std::slice::from_ref(&to),
                &body,
                &content_type,
                reply_to.as_deref(),
                conversation_id.as_deref(),
                correlation_id.as_deref(),
                None,
                metadata.as_deref(),
                &attachments,
            ) {
                Ok(message) => {
                    if let Ok(Some(participant)) = storage.get_participant_by_name(&to) {
                        notify_poll_waiter(storage, poll_waiters, &participant.id);
                        if let Some(transport) = transports.get(&participant.transport) {
                            let _ = transport
                                .deliver(&message.id, &sender, &body, &participant.transport_config)
                                .await;
                            let _ = storage.mark_delivered(&message.id, &to);
                        }
                    }

                    // 回复消息时隐式标记原消息为已读
                    if let Some(ref reply_to_id) = reply_to {
                        let _ = storage.mark_read(
                            reply_to_id,
                            &sender,
                            session_id_from(session).as_deref(),
                        );
                    }

                    // daemon 内执行 notify（按目标 active session 的 notify_config）
                    let mut notify_status = serde_json::json!({
                        "attempted": false,
                        "delivered": false,
                        "error": serde_json::Value::Null,
                    });
                    if notify {
                        notify_status["attempted"] = serde_json::Value::Bool(true);
                        match try_notify(
                            storage,
                            notify_plugins,
                            &sender,
                            &to,
                            &body,
                            &message.id,
                            send_enter,
                        )
                        .await
                        {
                            Ok(()) => {
                                notify_status["delivered"] = serde_json::Value::Bool(true);
                                let _ = storage.mark_delivered(&message.id, &to);
                            }
                            Err(e) => {
                                notify_status["error"] = serde_json::Value::String(e);
                            }
                        }
                    }

                    ServerMsg::Ok {
                        data: serde_json::json!({
                            "message": message,
                            "notify": notify_status,
                        }),
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "send_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Inbox {
            sender: _sender,
            participant,
            status,
            limit,
            peek,
        } => {
            match storage.list_inbox(&participant, status.as_deref(), limit) {
                Ok(items) => {
                    // 非 peek 时批量标记本次返回的消息为已读
                    if !peek {
                        let msg_ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
                        let _ = storage.mark_messages_read(
                            &msg_ids,
                            &participant,
                            session_id_from(session).as_deref(),
                        );
                    }
                    ServerMsg::Ok {
                        data: serde_json::to_value(&items).unwrap_or_default(),
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "inbox_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Done {
            sender,
            msg_id,
            participant,
            attachments,
        } => {
            let _sender = sender_from(session, sender);
            match storage.mark_done(
                &msg_id,
                &participant,
                session_id_from(session).as_deref(),
                &attachments,
            ) {
                Ok(()) => ServerMsg::Ok {
                    data: serde_json::json!({"msg_id": msg_id}),
                },
                Err(e) => ServerMsg::Error {
                    code: "done_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Register {
            name,
            participant_type,
            display_name,
            transport,
            transport_config,
        } => {
            let tc = transport_config.to_string();
            match storage.register_participant(
                None,
                &name,
                &participant_type,
                &display_name,
                &transport,
                &tc,
                "",
                "agent",
            ) {
                Ok(p) => ServerMsg::Ok {
                    data: serde_json::to_value(&p).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "register_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Unregister { name } => match storage.unregister_participant(&name) {
            Ok(()) => ServerMsg::Ok {
                data: serde_json::json!({"name": name}),
            },
            Err(e) => ServerMsg::Error {
                code: "unregister_failed".into(),
                message: e.to_string(),
            },
        },

        ClientMsg::ListParticipants { participant_type } => {
            match storage.list_peers(participant_type.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "list_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::ListConversations { participant } => {
            match storage.list_conversations(participant.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "list_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::GetMessages {
            conversation_id,
            limit,
            before,
            participant,
        } => {
            match storage.get_messages(&conversation_id, limit, before.as_deref()) {
                Ok(mut list) => {
                    // 进入 chat detail，自动把当前 viewer 的未读消息标为 read
                    let viewer = viewer_from(session, participant);
                    let session_id = session_id_from(session);
                    let unread_ids: Vec<String> = list
                        .iter()
                        .filter(|m| {
                            m.sender_name != viewer
                                && m.recipients.iter().any(|r| {
                                    r.recipient_name == viewer
                                        && r.status != "done"
                                        && r.read_at.is_none()
                                })
                        })
                        .map(|m| m.id.clone())
                        .collect();
                    if !unread_ids.is_empty() {
                        let _ =
                            storage.mark_messages_read(&unread_ids, &viewer, session_id.as_deref());
                        // 刷新 recipients 状态以返回最新值
                        for msg in &mut list {
                            msg.recipients = match storage.get_recipients_for_msg_by_id(&msg.id) {
                                Ok(r) => r,
                                Err(_) => msg.recipients.clone(),
                            };
                        }
                    }
                    ServerMsg::Ok {
                        data: serde_json::to_value(&list).unwrap_or_default(),
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "get_messages_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::GetMessage { msg_id, participant } => {
            let viewer = viewer_from(session, participant);
            match storage.get_message_by_id(
                &msg_id,
                Some(&viewer),
                session_id_from(session).as_deref(),
            ) {
                Ok(Some(msg)) => ServerMsg::Ok {
                    data: serde_json::to_value(&msg).unwrap_or_default(),
                },
                Ok(None) => ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("消息不存在: {}", msg_id),
                },
                Err(e) => ServerMsg::Error {
                    code: "get_message_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Detail { msg_id, participant } => {
            let viewer = viewer_from(session, participant);
            match storage.get_message_by_id(
                &msg_id,
                Some(&viewer),
                session_id_from(session).as_deref(),
            ) {
                Ok(Some(msg)) => ServerMsg::Ok {
                    data: serde_json::to_value(&msg).unwrap_or_default(),
                },
                Ok(None) => ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("消息不存在: {}", msg_id),
                },
                Err(e) => ServerMsg::Error {
                    code: "detail_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Attachment {
            attachment_id,
            participant,
        } => {
            let viewer = viewer_from(session, participant);
            match storage.get_attachment(
                &attachment_id,
                Some(&viewer),
                session_id_from(session).as_deref(),
            ) {
                Ok(Some((att, data))) => {
                    let content = String::from_utf8_lossy(&data).to_string();
                    ServerMsg::Ok {
                        data: serde_json::json!({
                            "attachment": att,
                            "content": content,
                        }),
                    }
                }
                Ok(None) => ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("附件不存在: {}", attachment_id),
                },
                Err(e) => ServerMsg::Error {
                    code: "attachment_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Read {
            sender,
            msg_id,
            participant,
        } => {
            let _sender = sender_from(session, sender);
            match storage.mark_read(&msg_id, &participant, session_id_from(session).as_deref()) {
                Ok(()) => ServerMsg::Ok {
                    data: serde_json::json!({"msg_id": msg_id}),
                },
                Err(e) => ServerMsg::Error {
                    code: "read_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::WhoAmI => ServerMsg::Ok {
            data: serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "socket": crate::paths::socket_path(),
                "participant": session.as_ref().map(|s| s.participant_name.clone()).unwrap_or_else(|| "anonymous".into()),
            }),
        },

        ClientMsg::CreateConversation {
            participants,
            title,
        } => ServerMsg::Ok {
            data: serde_json::json!({"participants": participants, "title": title, "status": "created"}),
        },

        // ── v0.3 mem 长期知识库 ───────────────────
        ClientMsg::MemTopicAdd {
            workspace_id,
            slug,
            title,
            summary,
            aliases,
            priority,
        } => {
            let actor = sender_from(session, None);
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.add_mem_topic(
                ws.as_deref(),
                &slug,
                &title,
                Some(summary.as_str()).filter(|s| !s.is_empty()),
                &aliases,
                priority,
                &actor,
            ) {
                Ok(topic) => ServerMsg::Ok {
                    data: serde_json::to_value(&topic).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_topic_add_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemTopicList { workspace_id, status } => {
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.list_mem_topics(ws.as_deref(), status.as_deref()) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_topic_list_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemTopicShow { workspace_id, slug } => {
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.get_mem_topic_by_slug(ws.as_deref(), &slug) {
                Ok(Some(topic)) => ServerMsg::Ok {
                    data: serde_json::to_value(&topic).unwrap_or_default(),
                },
                Ok(None) => ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("topic 不存在: {}", slug),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_topic_show_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemTopicUpdate {
            workspace_id,
            slug,
            title,
            summary,
            aliases,
            priority,
            status,
        } => {
            let actor = sender_from(session, None);
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.update_mem_topic(
                ws.as_deref(),
                &slug,
                title.as_deref(),
                summary.as_deref(),
                aliases,
                priority,
                status.as_deref(),
                &actor,
            ) {
                Ok(topic) => ServerMsg::Ok {
                    data: serde_json::to_value(&topic).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_topic_update_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        #[allow(clippy::too_many_arguments)]
        ClientMsg::MemAdd {
            workspace_id,
            item_type,
            title,
            content,
            summary,
            topic_slugs,
            tags,
            importance,
            confidence,
            source_type,
            source_ref,
        } => {
            let actor = sender_from(session, None);
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.add_mem_item(
                ws.as_deref(),
                &item_type,
                &title,
                &content,
                Some(summary.as_str()).filter(|s| !s.is_empty()),
                &topic_slugs,
                &tags,
                importance,
                &confidence,
                &actor,
                &source_type,
                &source_ref,
            ) {
                Ok(item) => ServerMsg::Ok {
                    data: serde_json::to_value(&item).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_add_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemShow { mem_id } => {
            let resolved = match storage.resolve_mem_item_id(&mem_id) {
                Ok(id) => id,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "not_found".into(),
                        message: e.to_string(),
                    }
                }
            };
            match storage.get_mem_item_by_id(&resolved) {
                Ok(Some(item)) => ServerMsg::Ok {
                    data: serde_json::to_value(&item).unwrap_or_default(),
                },
                Ok(None) => ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("memory 不存在: {}", mem_id),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_show_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        #[allow(clippy::too_many_arguments)]
        ClientMsg::MemUpdate {
            mem_id,
            title,
            content,
            summary,
            topic_slugs,
            tags,
            importance,
            status,
        } => {
            let actor = sender_from(session, None);
            let resolved = match storage.resolve_mem_item_id(&mem_id) {
                Ok(id) => id,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "not_found".into(),
                        message: e.to_string(),
                    }
                }
            };
            match storage.update_mem_item(
                &resolved,
                title.as_deref(),
                content.as_deref(),
                summary.as_deref(),
                topic_slugs,
                tags,
                importance,
                status.as_deref(),
                &actor,
            ) {
                Ok(item) => ServerMsg::Ok {
                    data: serde_json::to_value(&item).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_update_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemArchive { mem_id } => {
            let actor = sender_from(session, None);
            let resolved = match storage.resolve_mem_item_id(&mem_id) {
                Ok(id) => id,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "not_found".into(),
                        message: e.to_string(),
                    }
                }
            };
            match storage.archive_mem_item(&resolved, &actor) {
                Ok(item) => ServerMsg::Ok {
                    data: serde_json::to_value(&item).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_archive_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemPromote {
            source_type,
            source_ref,
            workspace_id,
            item_type,
            title,
            summary,
            topic_slugs,
            tags,
            importance,
            confidence,
        } => {
            let actor = sender_from(session, None);
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            let result = match source_type.as_str() {
                "message" => storage.promote_message_to_mem(
                    &source_ref,
                    ws.as_deref(),
                    &topic_slugs,
                    &item_type,
                    &title,
                    Some(summary.as_str()).filter(|s| !s.is_empty()),
                    &tags,
                    importance,
                    &confidence,
                    &actor,
                ),
                "artifact" => storage.promote_artifact_to_mem(
                    &source_ref,
                    ws.as_deref(),
                    &topic_slugs,
                    &item_type,
                    &title,
                    Some(summary.as_str()).filter(|s| !s.is_empty()),
                    &tags,
                    importance,
                    &confidence,
                    &actor,
                ),
                _ => {
                    return ServerMsg::Error {
                        code: "mem_promote_failed".into(),
                        message: format!("不支持的 source_type: {}", source_type),
                    }
                }
            };
            match result {
                Ok(item) => ServerMsg::Ok {
                    data: serde_json::to_value(&item).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_promote_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemSearch {
            workspace_id,
            query,
            topic_slugs,
            item_type,
            scope,
            limit,
        } => {
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.search_mem(
                ws.as_deref(),
                query.as_deref(),
                Some(topic_slugs).filter(|v| !v.is_empty()),
                item_type.as_deref(),
                scope.as_deref(),
                Some("active"),
                limit,
            ) {
                Ok(list) => ServerMsg::Ok {
                    data: serde_json::to_value(&list).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_search_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemPack {
            workspace_id,
            topic_slug,
            limit,
        } => {
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.pack_mem(ws.as_deref(), &topic_slug, limit) {
                Ok(pack) => ServerMsg::Ok {
                    data: serde_json::to_value(&pack).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_pack_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::MemList {
            workspace_id,
            topic_slug,
            item_type,
            scope,
            status,
            limit,
        } => {
            let ws = workspace_id.or_else(|| session.as_ref().map(|s| s.workspace_id.clone()));
            match storage.list_mem_items(
                ws.as_deref(),
                topic_slug.as_deref(),
                item_type.as_deref(),
                scope.as_deref(),
                &status,
                limit,
            ) {
                Ok(items) => ServerMsg::Ok {
                    data: serde_json::to_value(&items).unwrap_or_default(),
                },
                Err(e) => ServerMsg::Error {
                    code: "mem_list_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::PollInbox {
            filter,
            timeout_ms,
            limit,
        } => {
            let session_info = match session {
                Some(s) => s.clone(),
                None => {
                    return ServerMsg::Error {
                        code: "auth_required".into(),
                        message: "PollInbox 需要 session 鉴权".into(),
                    }
                }
            };

            // 参数规范化
            let timeout_ms = timeout_ms.clamp(100, 30000);
            let limit = if limit == 0 { 10 } else { limit.min(50) };
            let filter_str = filter.as_str();
            let participant_name = session_info.participant_name.clone();
            let participant_id = session_info.participant_id.clone();
            let workspace_id = session_info.workspace_id.clone();

            // 第一次查询
            match poll_inbox_once(
                storage,
                &participant_name,
                filter_str,
                limit,
            ) {
                Ok((items, _)) if !items.is_empty() => {
                    let result = crate::ipc::PollInboxResult {
                        empty: false,
                        timed_out: false,
                        limit,
                        timeout_ms,
                        messages: items,
                    };
                    return ServerMsg::Ok {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    };
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "poll_inbox_failed".into(),
                        message: e.to_string(),
                    }
                }
                Ok(_) => {}
            }

            // 没有消息：注册 waiter
            let key = PollKey {
                workspace_id: workspace_id.clone(),
                participant_id: participant_id.clone(),
            };
            let (tx, rx) = oneshot::channel();
            {
                let mut waiters = poll_waiters.lock().unwrap();
                if waiters.contains_key(&key) {
                    return ServerMsg::Error {
                        code: "poll_already_active".into(),
                        message: "当前 participant 已有一个挂起中的 PollInbox 请求".into(),
                    };
                }
                waiters.insert(key.clone(), tx);
            }
            // guard 确保 future 被 cancel / 正常返回 / panic 时都能清理 waiter
            let _guard = PollWaiterGuard {
                waiters: poll_waiters.clone(),
                key: key.clone(),
            };

            // 注册后再查一次，避免竞态
            match poll_inbox_once(
                storage,
                &participant_name,
                filter_str,
                limit,
            ) {
                Ok((items, _)) if !items.is_empty() => {
                    let result = crate::ipc::PollInboxResult {
                        empty: false,
                        timed_out: false,
                        limit,
                        timeout_ms,
                        messages: items,
                    };
                    return ServerMsg::Ok {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    };
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "poll_inbox_failed".into(),
                        message: e.to_string(),
                    };
                }
                Ok(_) => {}
            }

            // 等待唤醒或超时
            let timed_out = tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                rx,
            )
            .await
            .is_err();

            // 最后再查一次
            match poll_inbox_once(
                storage,
                &participant_name,
                filter_str,
                limit,
            ) {
                Ok((items, _)) => {
                    let empty = items.is_empty();
                    let result = crate::ipc::PollInboxResult {
                        empty,
                        timed_out,
                        limit,
                        timeout_ms,
                        messages: items,
                    };
                    ServerMsg::Ok {
                        data: serde_json::to_value(&result).unwrap_or_default(),
                    }
                }
                Err(e) => ServerMsg::Error {
                    code: "poll_inbox_failed".into(),
                    message: e.to_string(),
                },
            }
        }

        ClientMsg::Ping => ServerMsg::Ok {
            data: serde_json::json!({"pong": true}),
        },
    }
}
