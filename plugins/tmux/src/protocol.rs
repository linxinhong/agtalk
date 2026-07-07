//! agtalk notify plugin 共享协议类型。

use serde::{Deserialize, Serialize};

/// discover 子命令输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverOutput {
    pub version: u32,
    #[serde(rename = "type")]
    pub type_: String,
    pub channel: String,
    pub ready: bool,
    #[serde(default)]
    pub endpoint: serde_json::Value,
    #[serde(default)]
    pub message: String,
}

impl DiscoverOutput {
    pub fn ready(pane: impl Into<String>) -> Self {
        Self {
            version: 1,
            type_: "notify_endpoint".to_string(),
            channel: "tmux".to_string(),
            ready: true,
            endpoint: serde_json::json!({ "pane": pane.into() }),
            message: "tmux session available".to_string(),
        }
    }

    pub fn not_ready(message: impl Into<String>) -> Self {
        Self {
            version: 1,
            type_: "notify_endpoint".to_string(),
            channel: "tmux".to_string(),
            ready: false,
            endpoint: serde_json::Value::Null,
            message: message.into(),
        }
    }
}

/// send 子命令输入。
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SendInput {
    pub version: u32,
    #[serde(rename = "type")]
    pub type_: String,
    pub endpoint: serde_json::Value,
    pub from_name: String,
    pub read_command: String,
    #[serde(default)]
    pub read_args: Vec<String>,
    pub binary_path: String,
    pub workspace: String,
    pub agent_name: String,
    pub agent_address: String,
}

impl SendInput {
    pub fn pane(&self) -> Option<String> {
        self.endpoint.get("pane")?.as_str().map(|s| s.to_string())
    }
}
