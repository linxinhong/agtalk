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
        participant_type: Option<String>,
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
        #[serde(default)]
        takeover: bool,
    },
    /// 接管已有 peer 的身份，为其创建新 session
    Attach {
        workspace_root: String,
        workspace_name: String,
        name: String,
        #[serde(default)]
        notify_config: serde_json::Value,
        #[serde(default)]
        runtime_config: serde_json::Value,
        #[serde(default)]
        takeover: bool,
    },
    /// 清理 inactive session
    Cleanup {
        workspace_id: String,
        #[serde(default)]
        dry_run: bool,
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
        /// 是否包含已注销（软删除）的参与者
        #[serde(default)]
        include_deleted: bool,
        /// 是否只返回在线的参与者
        #[serde(default)]
        active_only: bool,
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
        #[serde(default)]
        participant: Option<String>,
    },
    /// 按 id 获取单条消息（审批弹窗用）
    GetMessage {
        msg_id: String,
        #[serde(default)]
        participant: Option<String>,
    },
    /// 查看消息详情（自动标记已读）
    Detail {
        msg_id: String,
        #[serde(default)]
        participant: Option<String>,
    },
    /// 读取附件全文（自动标记已读）
    Attachment {
        attachment_id: String,
        #[serde(default)]
        participant: Option<String>,
    },
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

    // ── v0.3 mem 长期知识库 ───────────────────
    MemTopicAdd {
        #[serde(default)]
        workspace_id: Option<String>,
        slug: String,
        title: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default = "default_priority")]
        priority: i32,
    },
    MemTopicList {
        #[serde(default)]
        workspace_id: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    MemTopicShow {
        #[serde(default)]
        workspace_id: Option<String>,
        slug: String,
    },
    MemTopicUpdate {
        #[serde(default)]
        workspace_id: Option<String>,
        slug: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        aliases: Option<Vec<String>>,
        #[serde(default)]
        priority: Option<i32>,
        #[serde(default)]
        status: Option<String>,
    },
    MemAdd {
        #[serde(default)]
        workspace_id: Option<String>,
        item_type: String,
        title: String,
        content: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        topic_slugs: Vec<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default = "default_priority")]
        importance: i32,
        #[serde(default = "default_confidence")]
        confidence: String,
        #[serde(default = "default_source_type")]
        source_type: String,
        #[serde(default)]
        source_ref: String,
    },
    MemShow {
        mem_id: String,
    },
    MemUpdate {
        mem_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        topic_slugs: Option<Vec<String>>,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        importance: Option<i32>,
        #[serde(default)]
        status: Option<String>,
    },
    MemArchive {
        mem_id: String,
    },
    MemPromote {
        source_type: String,
        source_ref: String,
        #[serde(default)]
        workspace_id: Option<String>,
        item_type: String,
        title: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        topic_slugs: Vec<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default = "default_priority")]
        importance: i32,
        #[serde(default = "default_confidence")]
        confidence: String,
    },
    MemSearch {
        #[serde(default)]
        workspace_id: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        topic_slugs: Vec<String>,
        #[serde(default)]
        item_type: Option<String>,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default = "default_limit")]
        limit: u32,
    },
    MemPack {
        #[serde(default)]
        workspace_id: Option<String>,
        topic_slug: String,
        #[serde(default = "default_limit")]
        limit: u32,
    },
    MemList {
        #[serde(default)]
        workspace_id: Option<String>,
        #[serde(default)]
        topic_slug: Option<String>,
        #[serde(default)]
        item_type: Option<String>,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default = "default_status_active")]
        status: String,
        #[serde(default = "default_limit")]
        limit: u32,
    },
    PollInbox {
        #[serde(default = "default_inbox_filter")]
        filter: InboxFilter,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
        #[serde(default = "default_limit")]
        limit: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxFilter {
    Unread,
    Pending,
    ActionRequired,
    All,
}

impl InboxFilter {
    pub fn as_str(&self) -> &'static str {
        match self {
            InboxFilter::Unread => "unread",
            InboxFilter::Pending => "pending",
            InboxFilter::ActionRequired => "action_required",
            InboxFilter::All => "all",
        }
    }
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
    /// 人类关闭弹窗，未作出选择
    AskDismissed { msg_id: String },
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct PollInboxResult {
    pub messages: Vec<crate::storage::InboxItem>,
    pub timed_out: bool,
    pub empty: bool,
    pub limit: u32,
    pub timeout_ms: u64,
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

fn default_priority() -> i32 {
    3
}

fn default_confidence() -> String {
    "confirmed".to_string()
}

fn default_source_type() -> String {
    "manual".to_string()
}

fn default_status_active() -> String {
    "active".to_string()
}

fn default_inbox_filter() -> InboxFilter {
    InboxFilter::Unread
}

fn default_timeout_ms() -> u64 {
    30000
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
