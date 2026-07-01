//! IPC 协议：ClientMsg / ServerMsg enum 定义。

use crate::identity::mailbox::Mailbox;
use crate::routing::Message;
use serde::{Deserialize, Serialize};

/// 扁平化的 send 请求体，供 `POST /api/send` 使用。
#[derive(Debug, Clone, Deserialize)]
pub struct SendPayload {
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub reply_to_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<String>,
    #[serde(default)]
    pub more_coming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Ping,
    Lookup {
        name: Option<String>,
    },
    Send {
        to: String,
        body: String,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        reply_to_id: Option<String>,
        #[serde(default)]
        metadata: Option<String>,
        #[serde(default)]
        more_coming: bool,
    },
    Inbox {
        #[serde(default)]
        include_done: bool,
    },
    Detail {
        message_id: String,
    },
    Reply {
        message_id: String,
        body: String,
        #[serde(default)]
        choice: Option<String>,
    },
    Join {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        intro: Option<String>,
        #[serde(default)]
        workspace: Option<String>,
        pid: u32,
        start_time: u64,
    },
    Leave {
        #[serde(default)]
        name: Option<String>,
        address: String,
    },
    Whoami,
    Human {
        body: String,
        #[serde(default)]
        choices: Option<Vec<String>>,
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
    LookupResult {
        mailboxes: Vec<Mailbox>,
    },
    InboxResult {
        messages: Vec<Message>,
    },
    MessageDetail(Message),
    WhoamiResult {
        address: String,
        name: String,
        workspace: String,
        intro: String,
    },
}
