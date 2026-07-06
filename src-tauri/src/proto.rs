//! IPC 协议：ClientMsg / ServerMsg enum 定义。

use crate::identity::mailbox::Mailbox;
use crate::routing::Message;
use serde::{Deserialize, Serialize};

pub fn default_notify() -> String {
    "auto".to_string()
}

/// 询问选项结构，用于 `MsgAsk`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<String>,
    #[serde(default)]
    pub single: bool,
    #[serde(default)]
    pub select_only: bool,
}

/// 收件箱过滤参数，用于 `MsgInbox`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InboxFilter {
    #[serde(default)]
    pub peek: bool,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub action_required: bool,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// 诊断检查项，用于 `ToolDiagnosis`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosisCheck {
    #[serde(default)]
    pub category: String,
    pub name: String,
    pub status: String, // ok / warn / error
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub details: serde_json::Value,
}

/// 聚合后的根因，用于 agent 快速决策。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RootCause {
    pub id: String,
    pub status: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// 诊断上下文，精简展示当前身份与消息状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosisContext {
    pub identity: Option<String>,
    pub address: Option<String>,
    pub pending: Option<i64>,
}

/// run 单步结果，用于 `RunResult`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStepResult {
    pub action: String,
    pub status: String, // ok / error
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Ping,

    // id
    IdJoin {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        intro: Option<String>,
        #[serde(default)]
        workspace: Option<String>,
        #[serde(default)]
        notify: String,
        pid: u32,
        start_time: u64,
    },
    IdShow,
    IdLookup {
        #[serde(default)]
        name: Option<String>,
    },
    IdLeave {
        #[serde(default)]
        purge: bool,
    },

    // msg
    MsgSend {
        to: String,
        body: String,
        #[serde(default)]
        subject: Option<String>,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        notify: bool,
        #[serde(default)]
        more: bool,
    },
    MsgReply {
        message_id: String,
        body: String,
        #[serde(default)]
        files: Vec<String>,
        #[serde(default)]
        notify: bool,
    },
    MsgDone {
        message_id: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        files: Vec<String>,
    },
    MsgAsk {
        message: String,
        #[serde(default)]
        options: AskOptions,
        #[serde(default)]
        wait: bool,
        #[serde(default)]
        timeout: Option<u64>,
    },
    MsgInbox {
        #[serde(default)]
        filter: InboxFilter,
    },
    MsgRead {
        #[serde(default)]
        message_id: Option<String>,
    },
    MsgWait {
        #[serde(default)]
        message_id: Option<String>,
        timeout: u64,
        #[serde(default)]
        since: Option<i64>,
    },
    MsgAttachment {
        attachment_id: String,
    },

    // mem
    MemPlanShow {
        #[serde(default)]
        target: Option<String>,
    },
    MemPlanUpdate {
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        summary: Option<String>,
    },
    MemPlanStatus {
        #[serde(default)]
        target: Option<String>,
    },
    MemAdd {
        text: String,
        topic: String,
        #[serde(rename = "entry_type")]
        ty: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    },
    MemSearch {
        query: String,
        #[serde(default)]
        topic: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    MemShow {
        id: String,
    },
    MemList {
        #[serde(default)]
        topic: Option<String>,
    },
    MemPack {
        topic: String,
        #[serde(default)]
        limit: Option<usize>,
    },

    // tool
    ToolDaemon {
        action: String,
    },
    ToolDoctor,
    ToolVersion,
    ToolPath,

    // config
    ConfigShow,
    ConfigGet {
        key: String,
    },
    ConfigSet {
        key: String,
        value: String,
    },
    ConfigPath,

    // run
    Run {
        #[serde(default)]
        file: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Pong,
    Ok {
        id: String,
    },
    Error {
        code: String,
        message: String,
    },

    // id
    Identity {
        address: String,
        name: String,
        workspace: String,
        intro: String,
    },
    LookupResult {
        mailboxes: Vec<Mailbox>,
    },

    // msg
    InboxResult {
        messages: Vec<Message>,
    },
    MsgDetail(Message),
    WaitResult {
        messages: Vec<Message>,
        body: String,
    },
    AskResult {
        message_id: String,
    },

    // mem
    MemPlanShow {
        address: String,
        name: String,
        plan: String,
        context: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        updated_at: String,
    },
    MemPlanStatus {
        address: String,
        name: String,
        #[serde(default)]
        updated_at: String,
        status: String,
        summary: String,
    },
    MemPackResult {
        topic: String,
        entries: Vec<serde_json::Value>,
    },
    MemPack {
        topic: String,
        markdown: String,
    },
    MemSearchResult {
        entries: Vec<serde_json::Value>,
    },
    MemShowResult {
        entry: serde_json::Value,
    },

    // config
    ConfigValue {
        key: String,
        value: serde_json::Value,
    },
    ConfigShowResult {
        config: serde_json::Value,
    },
    ConfigPath {
        path: String,
    },

    // tool
    ToolDiagnosis {
        status: String,
        summary: String,
        root_causes: Vec<RootCause>,
        actions: Vec<String>,
        context: DiagnosisContext,
        checks: Vec<DiagnosisCheck>,
    },
    ToolVersionInfo {
        version: String,
    },
    ToolPathInfo {
        path: String,
    },

    // run
    RunResult {
        steps: Vec<RunStepResult>,
    },

    // daemon
    DaemonStatus {
        pid: u32,
        #[serde(default)]
        start_time: u64,
        version: String,
        http_port: u16,
        uptime_seconds: u64,
        active_mailboxes: i64,
        pending_messages: i64,
        sse_subscribers: usize,
        config_path: String,
        db_path: String,
    },

    // browser extension
    BrowserJoinResult {
        address: String,
        name: String,
        token: String,
    },
}
