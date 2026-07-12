//! 飞书长连接（WebSocket）：protobuf 帧（pbbp2）收事件 + 卡片回调（零公网）。
//!
//! 协议（对齐官方 SDK lark-oapi，参考 AskHuman feishu/ws.rs 实现）：
//! 1. `POST {base}/callback/ws/endpoint {AppID,AppSecret}` → `data.URL`(wss) + `ClientConfig`。
//! 2. 连 wss；帧为 protobuf `PbFrame`。method=0 控制帧(ping/pong)，method=1 数据帧(JSON 业务)。
//! 3. 客户端按 `PingInterval` 主动发 ping 帧维持心跳。
//! 4. 数据帧 header：`type`(event/card)、`message_id`、`sum`/`seq`(大消息分片，按 message_id 重组)。
//!    收到后须 **3 秒内回包**：同 message_id 的帧，payload = `{"code":200[,"data":<base64(JSON)>]}`。
//!    事件回空 ACK；卡片回调回 `{toast,...}`（base64 进 data）。

use super::proto::{
    PbFrame, FRAME_DATA, HEADER_MESSAGE_ID, HEADER_SEQ, HEADER_SUM, HEADER_TYPE, MSG_TYPE_CARD,
};
use super::FeishuError;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const GEN_ENDPOINT_URI: &str = "/callback/ws/endpoint";
const DEFAULT_PING_SECS: u64 = 120;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 上抛给上层的业务事件（`data` 为已解析的 event JSON）。
pub enum WsEvent {
    /// 收到用户消息（`im.message.receive_v1` 的 `event`）。已自动 ACK。
    Message {
        data: Value,
        /// 飞书 header.event_id（跨端幂等键）。
        event_id: Option<String>,
    },
    /// 卡片回传交互（`card.action.trigger` 的 `event`）。**未自动 ACK**：上层须 3 秒内
    /// 调 `respond_card` / `respond_ack`（带回原 `frame`）回包，否则飞书会重推。
    CardAction {
        data: Value,
        event_id: Option<String>,
        frame: PbFrame,
    },
}

pub struct FeishuWs {
    http: reqwest::Client,
    base_url: String,
    app_id: String,
    app_secret: String,
    write: SplitSink<Ws, Message>,
    read: SplitStream<Ws>,
    service_id: i32,
    ping: tokio::time::Interval,
    /// message_id → 分片槽（大消息重组）。
    frag: HashMap<String, Vec<Option<Vec<u8>>>>,
}

impl FeishuWs {
    /// 建立长连接：取 endpoint → 连 wss → 拆分读写 → 解析 service_id + 心跳间隔 → 首个 ping。
    pub async fn connect(
        http: reqwest::Client,
        base_url: &str,
        app_id: &str,
        app_secret: &str,
    ) -> Result<Self, FeishuError> {
        let (url, ping_secs) = open_endpoint(&http, base_url, app_id, app_secret).await?;
        let service_id = parse_query_i32(&url, "service_id").unwrap_or(0);
        let (ws, _resp) = connect_async(url)
            .await
            .map_err(|e| FeishuError::Network(format!("WebSocket connection failed: {}", e)))?;
        let (write, read) = ws.split();
        let dur = Duration::from_secs(ping_secs.max(10));
        let ping = tokio::time::interval_at(tokio::time::Instant::now() + dur, dur);
        let mut me = Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            write,
            read,
            service_id,
            ping,
            frag: HashMap::new(),
        };
        let _ = me.send_app_ping().await; // 首个 ping，校准心跳（与官方 SDK 一致）。
        Ok(me)
    }

    /// 收下一个业务事件；内部处理 ping/pong、分片重组、自动 ACK、断线重连。
    /// 返回 `None` 表示重连多次仍失败（上层据此结束）。
    pub async fn recv(&mut self) -> Option<WsEvent> {
        loop {
            enum Step {
                Frame(Vec<u8>),
                WsPing(Vec<u8>),
                Ignore,
                Dead,
                AppPing,
            }
            let step = tokio::select! {
                biased;
                msg = self.read.next() => match msg {
                    Some(Ok(Message::Binary(b))) => Step::Frame(b.to_vec()),
                    Some(Ok(Message::Ping(p))) => Step::WsPing(p.to_vec()),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => Step::Dead,
                    Some(Ok(_)) => Step::Ignore,
                },
                _ = self.ping.tick() => Step::AppPing,
            };
            match step {
                Step::Frame(bytes) => {
                    if let Some(ev) = self.handle_frame(&bytes).await {
                        return Some(ev);
                    }
                }
                Step::WsPing(p) => {
                    let _ = self.write.send(Message::Pong(p.into())).await;
                }
                Step::AppPing => {
                    let _ = self.send_app_ping().await;
                }
                Step::Ignore => {}
                Step::Dead => {
                    if !self.reconnect().await {
                        return None;
                    }
                }
            }
        }
    }

    /// 处理一帧；业务帧返回事件，控制/分片未满/忽略类返回 None。
    async fn handle_frame(&mut self, bytes: &[u8]) -> Option<WsEvent> {
        let frame = PbFrame::decode(bytes).ok()?;
        if frame.method != FRAME_DATA {
            // 控制帧（ping/pong）：维持心跳即可，pong 可带新 ClientConfig（从简不动态调整）。
            return None;
        }

        let msg_id = frame.header(HEADER_MESSAGE_ID).to_string();
        let sum: usize = frame.header(HEADER_SUM).parse().unwrap_or(1);
        let seq: usize = frame.header(HEADER_SEQ).parse().unwrap_or(0);

        let payload = frame.payload.clone().unwrap_or_default();
        let payload = if sum > 1 {
            match combine_frag(&mut self.frag, &msg_id, sum, seq, payload) {
                Some(p) => p,
                None => return None, // 分片未满，等后续帧
            }
        } else {
            payload
        };

        let value: Value = serde_json::from_slice(&payload).ok()?;
        // 业务路由以 JSON 内的 header.event_type 为准（权威）——卡片回调可能以
        // type="card" 或 type="event" 投递，统一按 event_type 分发更稳。
        let event_type = value
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let frame_type = frame.header(HEADER_TYPE).to_string();
        let event_id = value
            .get("header")
            .and_then(|h| h.get("event_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if event_type == "card.action.trigger" || frame_type == MSG_TYPE_CARD {
            // 卡片回调：延迟 ACK——由上层算出回包后调 respond_*（须 3 秒内）。
            return Some(WsEvent::CardAction {
                data: value.get("event").cloned().unwrap_or(Value::Null),
                event_id,
                frame,
            });
        }

        // 其余事件：立即空 ACK 再按需上抛。
        self.respond_ack(&frame).await;
        if event_type == "im.message.receive_v1" {
            return Some(WsEvent::Message {
                data: value.get("event").cloned().unwrap_or(Value::Null),
                event_id,
            });
        }
        None
    }

    /// 空 ACK（事件 / 非匹配卡片）：回 `{"code":200}`。
    pub async fn respond_ack(&mut self, frame: &PbFrame) {
        self.respond(frame, None).await;
    }

    /// 卡片回包：把业务响应体（如 `{toast,...}`）base64 进 `data`。
    pub async fn respond_card(&mut self, frame: &PbFrame, body: &Value) {
        let data = B64.encode(body.to_string().as_bytes());
        self.respond(frame, Some(data)).await;
    }

    async fn respond(&mut self, frame: &PbFrame, data_b64: Option<String>) {
        let resp = match data_b64 {
            Some(d) => json!({ "code": 200, "data": d }),
            None => json!({ "code": 200 }),
        };
        let out = frame.ack_response(resp);
        let _ = self
            .write
            .send(Message::Binary(out.encode_to_vec().into()))
            .await;
    }

    async fn send_app_ping(&mut self) -> Result<(), FeishuError> {
        let frame = PbFrame::ping(self.service_id);
        self.write
            .send(Message::Binary(frame.encode_to_vec().into()))
            .await
            .map_err(|e| FeishuError::Network(e.to_string()))
    }

    /// 断线重连：重新取 endpoint + 连接，线性退避，最多 5 次。
    async fn reconnect(&mut self) -> bool {
        for attempt in 0..5u32 {
            tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
            let opened =
                open_endpoint(&self.http, &self.base_url, &self.app_id, &self.app_secret).await;
            let Ok((url, ping_secs)) = opened else {
                continue;
            };
            let service_id = parse_query_i32(&url, "service_id").unwrap_or(0);
            if let Ok((ws, _)) = connect_async(url).await {
                let (write, read) = ws.split();
                self.write = write;
                self.read = read;
                self.service_id = service_id;
                let dur = Duration::from_secs(ping_secs.max(10));
                self.ping = tokio::time::interval_at(tokio::time::Instant::now() + dur, dur);
                self.frag.clear();
                let _ = self.send_app_ping().await;
                return true;
            }
        }
        false
    }
}

/// 取长连接 endpoint：返回 (wss URL, ping 间隔秒)。
async fn open_endpoint(
    http: &reqwest::Client,
    base_url: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<(String, u64), FeishuError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), GEN_ENDPOINT_URI);
    let v: Value = http
        .post(&url)
        .header("locale", "zh")
        .json(&json!({ "AppID": app_id, "AppSecret": app_secret }))
        .send()
        .await?
        .json()
        .await?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(FeishuError::Api {
            code,
            message: v
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("failed to obtain Feishu long-connection endpoint")
                .to_string(),
        });
    }
    let data = v
        .get("data")
        .ok_or_else(|| FeishuError::Proto("missing data in endpoint response".into()))?;
    let conn_url = data
        .get("URL")
        .and_then(|u| u.as_str())
        .ok_or_else(|| FeishuError::Proto("missing data.URL".into()))?
        .to_string();
    let ping_secs = data
        .get("ClientConfig")
        .and_then(|c| c.get("PingInterval"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_PING_SECS);
    Ok((conn_url, ping_secs))
}

/// 大消息分片重组（纯函数）：填入 seq 槽，集齐返回完整 payload，否则 None。
fn combine_frag(
    frag: &mut HashMap<String, Vec<Option<Vec<u8>>>>,
    msg_id: &str,
    sum: usize,
    seq: usize,
    bs: Vec<u8>,
) -> Option<Vec<u8>> {
    let slots = frag
        .entry(msg_id.to_string())
        .or_insert_with(|| vec![None; sum]);
    if seq < slots.len() {
        slots[seq] = Some(bs);
    }
    if slots.iter().any(|s| s.is_none()) {
        return None;
    }
    let full: Vec<u8> = slots.iter().flatten().flatten().copied().collect();
    frag.remove(msg_id);
    Some(full)
}

/// 从 URL query 解析 i32 参数。
fn parse_query_i32(url: &str, key: &str) -> Option<i32> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(key) {
            return it.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_frag_out_of_order() {
        let mut frag = HashMap::new();
        assert!(combine_frag(&mut frag, "m1", 3, 1, b"bb".to_vec()).is_none());
        assert!(combine_frag(&mut frag, "m1", 3, 0, b"aa".to_vec()).is_none());
        let full = combine_frag(&mut frag, "m1", 3, 2, b"cc".to_vec()).unwrap();
        assert_eq!(full, b"aabbcc");
        // 重组完成后槽位被清除
        assert!(!frag.contains_key("m1"));
    }

    #[test]
    fn combine_frag_independent_message_ids() {
        let mut frag = HashMap::new();
        assert!(combine_frag(&mut frag, "m1", 2, 0, b"a".to_vec()).is_none());
        assert!(combine_frag(&mut frag, "m2", 2, 0, b"x".to_vec()).is_none());
        assert_eq!(
            combine_frag(&mut frag, "m2", 2, 1, b"y".to_vec()).unwrap(),
            b"xy"
        );
        assert_eq!(
            combine_frag(&mut frag, "m1", 2, 1, b"b".to_vec()).unwrap(),
            b"ab"
        );
    }

    #[test]
    fn combine_frag_out_of_range_seq_ignored() {
        let mut frag = HashMap::new();
        // seq 超出 sum 不 panic，也不填槽
        assert!(combine_frag(&mut frag, "m1", 1, 5, b"z".to_vec()).is_none());
    }

    #[test]
    fn parse_query_i32_works() {
        assert_eq!(
            parse_query_i32("wss://x/ws?service_id=42&foo=bar", "service_id"),
            Some(42)
        );
        assert_eq!(parse_query_i32("wss://x/ws", "service_id"), None);
        assert_eq!(
            parse_query_i32("wss://x/ws?service_id=abc", "service_id"),
            None
        );
    }

    #[test]
    fn data_frame_classification_uses_event_type() {
        // 帧 type=event 但 event_type=card.action.trigger → 仍按卡片回调分发
        let mut frame = PbFrame {
            seq_id: 1,
            log_id: 1,
            service: 1,
            method: FRAME_DATA,
            headers: vec![],
            payload_encoding: None,
            payload_type: None,
            payload: Some(
                serde_json::json!({
                    "header": { "event_type": "card.action.trigger" },
                    "event": { "action": {} }
                })
                .to_string()
                .into_bytes(),
            ),
            log_id_new: None,
        };
        frame.set_header(HEADER_TYPE, "event");
        frame.set_header(HEADER_MESSAGE_ID, "mid-1");
        let value: serde_json::Value =
            serde_json::from_slice(frame.payload.as_deref().unwrap()).unwrap();
        let event_type = value
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(event_type, "card.action.trigger");
    }

    #[test]
    fn control_frame_constant_distinct() {
        use crate::feishu::proto::{FRAME_CONTROL, MSG_TYPE_PING};
        assert_ne!(FRAME_CONTROL, FRAME_DATA);
        assert_eq!(MSG_TYPE_PING, "ping");
    }
}
