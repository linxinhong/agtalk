//! daemon 全局 FeishuRouter：单条长连接，入站事件 → 决策（decide）→ 回包/投递。
//!
//! - 卡片回调：动作分派（approval / compose_submit / reply_open / reply_submit），
//!   飞书 event_id 经 `human_action_receipts` 幂等；3 秒窗口内回包换终态卡片。
//! - p2p 文本：回复 compose 草稿卡（预填正文 + agent 下拉）；群聊/非文本/空文本忽略。
//! - v1 单用户：仅 `config.feishu.open_id` 绑定用户的点击/消息被处理。

use super::ws::{FeishuWs, WsEvent};
use super::{card, client::FeishuClient};
use crate::config::FeishuConfig;
use crate::human::popup::PopupTransport;
use crate::notify::NotifyLimiter;
use crate::routing::Message;
use crate::server::handlers::msg as msg_handlers;
use crate::storage::Storage;
use crate::transport::wake::{SseEvent, SubscriberRegistry};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

pub const SURFACE: &str = "feishu";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// 长连接状态句柄（doctor 可读；daemon 与 Router 共享）。
#[derive(Clone, Default)]
pub struct LinkStatus(Arc<AtomicBool>);

impl LinkStatus {
    pub fn is_connected(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn set(&self, connected: bool) {
        self.0.store(connected, Ordering::Relaxed);
    }
}

pub struct FeishuRouter {
    storage: Storage,
    cfg: FeishuConfig,
    client: FeishuClient,
    link: LinkStatus,
    registry: SubscriberRegistry,
    notify_limiter: Arc<NotifyLimiter>,
    popup: Arc<PopupTransport>,
    dot_agtalk: PathBuf,
}

impl FeishuRouter {
    pub fn new(
        storage: Storage,
        cfg: FeishuConfig,
        link: LinkStatus,
        registry: SubscriberRegistry,
        notify_limiter: Arc<NotifyLimiter>,
        popup: Arc<PopupTransport>,
        dot_agtalk: PathBuf,
    ) -> Self {
        let client = FeishuClient::new(&cfg.base_url, &cfg.app_id, &cfg.app_secret);
        Self {
            storage,
            cfg,
            client,
            link,
            registry,
            notify_limiter,
            popup,
            dot_agtalk,
        }
    }

    /// 主循环：连接 → 收事件 → 断线重连（无限重试，退避固定 5s；
    /// ws.recv 内部还有连接级重试）。
    pub async fn run(self: Arc<Self>) {
        let http = reqwest::Client::new();
        loop {
            match FeishuWs::connect(
                http.clone(),
                &self.cfg.base_url,
                &self.cfg.app_id,
                &self.cfg.app_secret,
            )
            .await
            {
                Err(e) => {
                    warn!("feishu 长连接建立失败: {}", e);
                    self.link.set(false);
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                Ok(mut ws) => {
                    info!("feishu 长连接已建立");
                    self.link.set(true);
                    while let Some(ev) = ws.recv().await {
                        match ev {
                            WsEvent::CardAction {
                                data,
                                event_id,
                                frame,
                            } => {
                                let out = decide_card_action(
                                    &self.storage,
                                    &self.cfg.open_id,
                                    &data,
                                    event_id.as_deref(),
                                );
                                // 新消息落库后唤醒接收方（与 popup/GUI 回复同一套机制）
                                if let Some(msg) = &out.created {
                                    self.wake_recipient(msg);
                                }
                                // 抢答收尾：本端处理了 human 原消息，关闭对应的桌面弹窗
                                if let Some(original_id) = &out.settled {
                                    self.popup.settle(original_id);
                                }
                                match out.decision {
                                    CardDecision::Ack => ws.respond_ack(&frame).await,
                                    CardDecision::TerminalCard(card) => {
                                        // 回包必须包 card:{type:"raw"} 才会原地更新卡片
                                        ws.respond_card(&frame, &card::callback_update_card(card))
                                            .await
                                    }
                                }
                            }
                            WsEvent::Message { data, .. } => {
                                // 绑定发现：未配置 open_id 时日志输出发送者，便于首次配置
                                if self.cfg.open_id.is_empty() {
                                    if let Some(oid) = data
                                        .pointer("/sender/sender_id/open_id")
                                        .and_then(|v| v.as_str())
                                    {
                                        info!(
                                            "feishu 收到消息来自 open_id: {}（未绑定，可用 agtalk config set feishu.open_id {} 绑定）",
                                            oid, oid
                                        );
                                    }
                                } else if let InboundMessage::Compose { open_id, text } =
                                    decide_message(&self.cfg.open_id, &data)
                                {
                                    self.send_compose_card(&open_id, &text).await;
                                }
                            }
                        }
                    }
                    self.link.set(false);
                    warn!("feishu 长连接断开，准备重连");
                }
            }
        }
    }

    /// p2p 文本入站：回复 compose 草稿卡（预填正文 + agent 下拉）。
    /// 无可投递 agent 时回说明文本，不生成空选择卡片。
    async fn send_compose_card(&self, open_id: &str, draft: &str) {
        let agents = active_compose_agents(&self.storage);
        if agents.is_empty() {
            if let Err(e) = self
                .client
                .send_text(
                    open_id,
                    "[agtalk] 当前没有可投递的 agent，请先用 agtalk id join 注册。",
                )
                .await
            {
                warn!("feishu 空 agent 提示发送失败: {}", e);
            }
            return;
        }
        let card = card::compose_card(draft, &agents);
        if let Err(e) = self.client.send_card(open_id, &card).await {
            warn!("feishu compose 卡片发送失败: {}", e);
        }
    }

    /// 回复落库后唤醒接收方：SSE 推送 + notify 打扰 + 本地 history 记录。
    /// 与 popup/GUI 回复路径（server::handlers::human::after_human_reply）保持同一套机制，
    /// 否则飞书点击产生的 approval_response 只落库不推送，agent 侧 msg wait 会超时。
    fn wake_recipient(&self, msg: &Message) {
        self.registry.notify(
            &msg.to_address,
            SseEvent {
                message: msg.clone(),
            },
        );
        let storage = self.storage.clone();
        let to = msg.to_address.clone();
        let message_id = msg.id.clone();
        let limiter = self.notify_limiter.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::notify::trigger(&storage, &to, "human", &message_id, &limiter, None).await
            {
                tracing::debug!("notify trigger skipped: {}", e);
            }
        });
        if let Some(root) =
            msg_handlers::participant_root(&self.storage, &self.dot_agtalk, &msg.to_address)
        {
            if let Err(e) = crate::identity::history::append_message(
                &root,
                &msg.to_name,
                &msg.to_address,
                "in",
                msg,
                "human.reply",
            ) {
                warn!("receiver history append failed: {}", e);
            }
        }
    }
}

mod decide;
use decide::active_compose_agents;
pub use decide::{
    decide_card_action, decide_message, CardActionOutcome, CardDecision, InboundMessage,
};
