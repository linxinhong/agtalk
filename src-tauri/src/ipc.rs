//! IPC 协议定义：daemon 与 CLI/GUI 之间的消息格式。
//!
//! 使用换行分隔的 JSON（每行一条消息），通过 Unix domain socket 传输。

use serde::{Deserialize, Serialize};

/// CLI/GUI → Daemon 的请求消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// 连接认证：客户端连接后应首先发送，daemon 校验 session_id + token
    Auth { session_id: String, token: String },
    /// 加入 workspace 并创建 session
    Join {
        workspace_root: String,
        workspace_name: String,
        name: String,
        #[serde(default)]
        role: String,
        #[serde(default)]
        intro: String,
        #[serde(default = "default_transport")]
        transport: String,
        #[serde(default)]
        notify_config: serde_json::Value,
        #[serde(default)]
        runtime_config: serde_json::Value,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    /// 离开当前 session
    Leave {
        #[serde(default)]
        session_id: Option<String>,
    },
    /// 发送消息
    Send {
        #[serde(default)]
        sender: Option<String>,
        to: String,
        body: String,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default)]
        reply_to: Option<String>,
        #[serde(default)]
        correlation_id: Option<String>,
        #[serde(default = "default_content_type")]
        content_type: String,
        #[serde(default)]
        metadata: Option<serde_json::Value>,
        #[serde(default)]
        notify: bool,
        #[serde(default)]
        send_enter: Option<bool>,
        #[serde(default)]
        attachments: Vec<SendAttachment>,
    },
    /// 获取收件箱（某参与者的消息列表）
    Inbox {
        #[serde(default)]
        sender: Option<String>,
        participant: String,
        #[serde(default)]
        status: Option<String>,
        #[serde(default = "default_limit")]
        limit: u32,
        #[serde(default)]
        peek: bool,
    },
    /// 标记消息完成
    Done {
        #[serde(default)]
        sender: Option<String>,
        msg_id: String,
        participant: String,
        #[serde(default)]
        attachments: Vec<SendAttachment>,
    },
    /// 注册参与者
    Register {
        name: String,
        #[serde(default)]
        participant_type: String,
        #[serde(default)]
        display_name: String,
        #[serde(default)]
        transport: String,
        #[serde(default)]
        transport_config: serde_json::Value,
    },
    /// 注销参与者
    Unregister { name: String },
    /// 列出参与者
    ListParticipants {
        #[serde(default)]
        participant_type: Option<String>,
    },
    /// 列出对话
    ListConversations {
        #[serde(default)]
        participant: Option<String>,
    },
    /// 获取对话消息
    GetMessages {
        conversation_id: String,
        #[serde(default = "default_limit")]
        limit: u32,
        #[serde(default)]
        before: Option<String>,
    },
    /// 按 id 获取单条消息（审批弹窗用）
    GetMessage { msg_id: String },
    /// 查看消息详情（自动标记已读）
    Detail { msg_id: String },
    /// 读取附件全文（自动标记已读）
    Attachment { attachment_id: String },
    /// 获取参与者信息
    WhoAmI,
    /// 创建对话
    CreateConversation {
        participants: Vec<String>,
        #[serde(default)]
        title: Option<String>,
    },
    /// 标记消息已读
    Read {
        #[serde(default)]
        sender: Option<String>,
        msg_id: String,
        participant: String,
    },
    /// 心跳
    Ping,

    // ── v0.2 Human-in-the-loop ────────────────
    /// 发起阻塞式审批请求：CLI 会一直等到人类回复或超时
    Ask {
        #[serde(default)]
        sender: Option<String>,
        to: String,
        body: String,
        choices: Vec<String>,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    /// 回复审批请求
    Reply {
        #[serde(default)]
        sender: Option<String>,
        msg_id: String,
        choice: String,
        #[serde(default)]
        reason: String,
    },
    /// 等待审批结果（阻塞式长轮询，timeout 秒后返回）
    Wait {
        #[serde(default)]
        sender: Option<String>,
        msg_id: String,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
}

/// Daemon → CLI/GUI 的响应消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 成功响应（通用）
    Ok {
        #[serde(default)]
        data: serde_json::Value,
    },
    /// 错误响应
    Error { code: String, message: String },
    /// 事件推送（如新消息通知）
    Event {
        event: String,
        #[serde(default)]
        data: serde_json::Value,
    },

    // ── v0.2 Human-in-the-loop ────────────────
    /// 审批请求的回应
    AskResponse {
        msg_id: String,
        choice: String,
        #[serde(default)]
        reason: String,
    },
    /// 审批请求超时
    AskTimeout { msg_id: String },
    /// 等待审批的结果（Wait 命令返回）
    WaitResult {
        msg_id: String,
        status: String,
        #[serde(default)]
        choice: String,
        #[serde(default)]
        reason: String,
        #[serde(default)]
        timed_out: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendAttachment {
    pub path: String,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

fn default_content_type() -> String {
    "text".to_string()
}

fn default_transport() -> String {
    "terminal".to_string()
}

fn default_limit() -> u32 {
    50
}

fn default_timeout() -> u64 {
    300
}

/// 序列化一条 IPC 消息为 JSON 行
pub fn serialize<T: Serialize>(msg: &T) -> String {
    let mut json = serde_json::to_string(msg).unwrap_or_default();
    json.push('\n');
    json
}

/// 尝试从字节中反序列化一条 IPC 消息（期望换行分隔）
pub fn deserialize<T: serde::de::DeserializeOwned>(line: &str) -> Option<T> {
    serde_json::from_str(line.trim()).ok()
}
