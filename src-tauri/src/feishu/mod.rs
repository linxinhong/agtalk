//! 飞书 surface：内置 human transport（不是 notify plugin）。
//!
//! 架构（对齐 docs/design.md Phase 3 与 docs/askhuman-research.md 结论）：
//! - 纯协议层在本模块；daemon 全局 FeishuRouter（router.rs）持有单条长连接。
//! - 入站事件先落 human mailbox DB 再走 SSE/仲裁，飞书 event_id 经
//!   `human_action_receipts` 幂等；不起第二 daemon、不引入第二真相源。
//! - 长连接帧为自研 protobuf（pbbp2，prost derive），不引 lark-oapi SDK。
//! - v1 单用户模型：仅 `config.feishu.open_id` 一个允许的对话对象。

pub mod card;
pub mod client;
pub mod dispatch;
pub mod proto;
pub mod router;
pub mod setup;
pub mod token;
pub mod ws;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FeishuError {
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("网络错误: {0}")]
    Network(String),
    #[error("飞书 API 错误 {code}: {message}")]
    Api { code: i64, message: String },
    #[error("协议错误: {0}")]
    Proto(String),
    #[error("配置缺失: {0}")]
    Config(String),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}
