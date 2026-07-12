//! 飞书长连接帧（pbbp2.proto）：用 `#[derive(prost::Message)]` 直接编解码，无需 protoc。
//! 协议对齐官方 SDK lark-oapi 的帧结构，仅保留 agtalk 需要的字段。

use prost::Message as ProstMessage;

/// method=0 控制帧（ping/pong）。
pub const FRAME_CONTROL: i32 = 0;
/// method=1 数据帧（JSON 业务）。
pub const FRAME_DATA: i32 = 1;

pub const HEADER_TYPE: &str = "type";
pub const HEADER_MESSAGE_ID: &str = "message_id";
pub const HEADER_SUM: &str = "sum";
pub const HEADER_SEQ: &str = "seq";

/// 帧 `type` header 取值。业务路由以回包内 header.event_type 为准
/// （卡片回调实测可能以 type=event 投递），type=card 仅作兜底。
pub const MSG_TYPE_CARD: &str = "card";
pub const MSG_TYPE_PING: &str = "ping";
pub const MSG_TYPE_PONG: &str = "pong";

#[derive(Clone, PartialEq, ProstMessage)]
pub struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
pub struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

impl PbFrame {
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        if let Some(h) = self.headers.iter_mut().find(|h| h.key == key) {
            h.value = value.to_string();
        } else {
            self.headers.push(PbHeader {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }

    /// 构造控制帧（ping）。
    pub fn ping(service: i32) -> Self {
        let mut frame = Self {
            seq_id: 0,
            log_id: 0,
            service,
            method: FRAME_CONTROL,
            headers: vec![],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        };
        frame.set_header(HEADER_TYPE, MSG_TYPE_PING);
        frame
    }

    /// 构造数据帧的回包（ACK / 卡片响应）：克隆原帧保留 headers/service，
    /// 仅替换 payload 为响应 JSON（对齐飞书 3 秒回包协议）。
    pub fn ack_response(&self, payload: serde_json::Value) -> Self {
        let mut out = self.clone();
        out.payload = Some(payload.to_string().into_bytes());
        out.payload_encoding = None;
        out.payload_type = None;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbframe_encode_decode_roundtrip() {
        let mut frame = PbFrame::ping(7);
        frame.seq_id = 42;
        frame.payload = Some(b"hello".to_vec());
        let bytes = frame.encode_to_vec();
        let decoded = PbFrame::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.seq_id, 42);
        assert_eq!(decoded.service, 7);
        assert_eq!(decoded.method, FRAME_CONTROL);
        assert_eq!(decoded.header(HEADER_TYPE), MSG_TYPE_PING);
        assert_eq!(decoded.payload.as_deref(), Some(b"hello".as_slice()));
    }

    #[test]
    fn header_lookup_missing_returns_empty() {
        let frame = PbFrame::ping(1);
        assert_eq!(frame.header("no-such-key"), "");
    }

    #[test]
    fn ack_response_carries_message_id_and_json_payload() {
        let mut data = PbFrame {
            seq_id: 9,
            log_id: 3,
            service: 2,
            method: FRAME_DATA,
            headers: vec![],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        };
        data.set_header(HEADER_MESSAGE_ID, "msg-123");
        let ack = data.ack_response(serde_json::json!({ "code": 200 }));
        assert_eq!(ack.method, FRAME_DATA);
        assert_eq!(ack.service, 2);
        assert_eq!(ack.header(HEADER_MESSAGE_ID), "msg-123");
        let payload: serde_json::Value =
            serde_json::from_slice(ack.payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["code"], 200);
    }
}
