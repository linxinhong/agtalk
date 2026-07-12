//! 飞书 OpenAPI 薄客户端：发卡片/发文本/改卡片。
//! 鉴权统一 `tenant_access_token`（Bearer）；发消息 `receive_id_type=open_id`。
//! 互动卡片直接以 JSON 下发（`msg_type=interactive`，content 即卡片 JSON 字符串）。

use super::token::TokenCache;
use super::FeishuError;
use serde_json::{json, Value};

pub struct FeishuClient {
    http: reqwest::Client,
    base_url: String,
    tokens: TokenCache,
}

impl FeishuClient {
    pub fn new(base_url: &str, app_id: &str, app_secret: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            tokens: TokenCache::new(base_url, app_id, app_secret),
        }
    }

    /// 发送互动卡片，返回 open_message_id（后续 patch_card 收尾用）。
    pub async fn send_card(&self, open_id: &str, card: &Value) -> Result<String, FeishuError> {
        self.send_message(open_id, "interactive", card).await
    }

    /// 发送纯文本消息，返回 open_message_id。
    pub async fn send_text(&self, open_id: &str, text: &str) -> Result<String, FeishuError> {
        self.send_message(open_id, "text", &json!({ "text": text }))
            .await
    }

    /// 修改已发出的卡片（终态回写：「已由 X 处理」）。
    pub async fn patch_card(&self, open_message_id: &str, card: &Value) -> Result<(), FeishuError> {
        let token = self.tokens.tenant_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.base_url, open_message_id);
        let v = self
            .http
            .patch(&url)
            .bearer_auth(token)
            .json(&json!({ "content": card.to_string() }))
            .send()
            .await?
            .json::<Value>()
            .await?;
        check_envelope(&v)?;
        Ok(())
    }

    async fn send_message(
        &self,
        open_id: &str,
        msg_type: &str,
        content: &Value,
    ) -> Result<String, FeishuError> {
        let token = self.tokens.tenant_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type=open_id", self.base_url);
        let v = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "receive_id": open_id,
                "msg_type": msg_type,
                "content": content.to_string(),
            }))
            .send()
            .await?
            .json::<Value>()
            .await?;
        check_envelope(&v)?;
        v.get("data")
            .and_then(|d| d.get("message_id"))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| FeishuError::Proto("missing data.message_id".into()))
    }
}

/// 飞书 API 信封校验：code==0 为成功。
fn check_envelope(v: &Value) -> Result<(), FeishuError> {
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        return Err(FeishuError::Api {
            code,
            message: v
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("feishu api failed")
                .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mock_http_server;

    fn token_ok() -> String {
        serde_json::json!({
            "code": 0, "msg": "ok", "tenant_access_token": "t-1", "expire": 7200
        })
        .to_string()
    }

    #[tokio::test]
    async fn send_card_posts_interactive_and_returns_message_id() {
        let sent = serde_json::json!({
            "code": 0, "msg": "ok", "data": { "message_id": "om_abc123" }
        })
        .to_string();
        let (base, rx, handle) = mock_http_server(vec![token_ok(), sent]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        let card = json!({ "elements": [] });
        let id = client.send_card("ou_user", &card).await.unwrap();
        assert_eq!(id, "om_abc123");
        let req = rx.recv().unwrap();
        assert!(
            req.starts_with("POST /auth/v3/tenant_access_token/internal"),
            "{}",
            req
        );
        let req = rx.recv().unwrap();
        assert!(
            req.starts_with("POST /im/v1/messages?receive_id_type=open_id"),
            "{}",
            req
        );
        assert!(req.contains("\"msg_type\":\"interactive\""), "{}", req);
        assert!(req.contains("\"receive_id\":\"ou_user\""), "{}", req);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn send_text_wraps_text_content() {
        let sent = serde_json::json!({
            "code": 0, "msg": "ok", "data": { "message_id": "om_txt" }
        })
        .to_string();
        let (base, rx, handle) = mock_http_server(vec![token_ok(), sent]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        client.send_text("ou_user", "hello").await.unwrap();
        let _ = rx.recv().unwrap();
        let req = rx.recv().unwrap();
        assert!(req.contains("\"msg_type\":\"text\""), "{}", req);
        // content 为 JSON 字符串，内含转义后的 text 字段
        assert!(req.contains("\\\"text\\\":\\\"hello\\\""), "{}", req);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn patch_card_uses_message_id_path() {
        let ok = serde_json::json!({ "code": 0, "msg": "ok" }).to_string();
        let (base, rx, handle) = mock_http_server(vec![token_ok(), ok]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        client
            .patch_card("om_abc123", &json!({ "elements": [] }))
            .await
            .unwrap();
        let _ = rx.recv().unwrap();
        let req = rx.recv().unwrap();
        assert!(
            req.starts_with("PATCH /im/v1/messages/om_abc123"),
            "{}",
            req
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn api_error_propagates_code_and_msg() {
        let err = serde_json::json!({ "code": 230002, "msg": "open_id invalid" }).to_string();
        let (base, _rx, handle) = mock_http_server(vec![token_ok(), err]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        let e = client.send_card("ou_bad", &json!({})).await.unwrap_err();
        assert!(e.to_string().contains("230002"), "{}", e);
        assert!(e.to_string().contains("open_id invalid"), "{}", e);
        handle.join().unwrap();
    }
}
