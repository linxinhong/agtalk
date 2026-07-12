//! 飞书一键创建应用：OAuth 2.0 Device Authorization Grant（RFC 8628）薄实现。
//!
//! 流程（与 larksuite/node-sdk scene/registration 一致）：
//! 1. begin：`POST <accounts>/oauth/v1/app/registration`，form 参数
//!    `action=begin&archetype=PersonalAgent&auth_method=client_secret&request_user_info=open_id`，
//!    返回 device_code 与 verification_uri_complete。
//! 2. 授权链接 = verification_uri_complete + from/source/tp + addons（最小权限清单，
//!    gzip+base64url）+ createOnly=true + name=agtalk。
//! 3. poll：`action=poll&device_code=X`，pending/slow_down 继续，成功返回
//!    client_id/client_secret/user_info.open_id，失败 access_denied/expired_token。
//!
//! 端点无需任何凭证；Lark 国际租户在 poll 返回 tenant_brand=lark 后由调用方切换域名。

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Write;

/// 生产端点（accounts 域名，与 open-apis 不同域）。
pub const ACCOUNTS_BASE_URL: &str = "https://accounts.feishu.cn";
/// Lark 国际版域名（poll 返回 tenant_brand=lark 时切换，仅一次）。
pub const LARK_ACCOUNTS_BASE_URL: &str = "https://accounts.larksuite.com";

const ENDPOINT: &str = "/oauth/v1/app/registration";

/// begin 结果：授权链接 + 轮询参数。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetupBegin {
    pub url: String,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in: u64,
}

/// poll 结果：前端按此驱动轮询节奏。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SetupPoll {
    Pending,
    SlowDown {
        interval_secs: u64,
    },
    Success {
        app_id: String,
        app_secret: String,
        open_id: String,
        /// "feishu" / "lark"，lark 时调用方应切换 poll 域名后重试
        tenant_brand: Option<String>,
    },
    Denied,
    Expired,
}

#[derive(Deserialize)]
struct BeginResponse {
    device_code: String,
    verification_uri_complete: String,
    interval: Option<u64>,
    expires_in: Option<u64>,
}

/// 最小权限 addons：JSON → gzip → base64url（无填充）。
/// preset=false 表示丢弃平台默认模板（约 30 项权限），只保留机器人能力 + 下列增量。
pub fn build_addons() -> String {
    let addons = json!({
        "preset": false,
        "scopes": { "tenant": [
            "im:message:send_as_bot",
            "im:message:update",
            "im:message.p2p_msg:readonly"
        ]},
        "events": { "items": { "tenant": ["im.message.receive_v1"] } },
        "callbacks": { "items": ["card.action.trigger"] }
    });
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(addons.to_string().as_bytes())
        .expect("gzip write to Vec cannot fail");
    let gz = encoder.finish().expect("gzip finish on Vec cannot fail");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(gz)
}

/// 拼接授权链接：from/source/tp 标识 + addons + createOnly + 应用名。
/// verification_uri_complete 平台保证已含 `?user_code=...`；addons 为 URL-safe 字符集，
/// 其余参数均为安全 ASCII，直接拼接即可。
pub fn build_setup_url(verification_uri_complete: &str) -> String {
    format!(
        "{}&from=sdk&source=agtalk&tp=sdk&addons={}&createOnly=true&name=agtalk",
        verification_uri_complete,
        build_addons()
    )
}

async fn post_form(base_url: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), ENDPOINT);
    let resp = reqwest::Client::new()
        .post(&url)
        .form(params)
        .send()
        .await
        .map_err(|e| format!("请求飞书注册端点失败: {}", e))?;
    // RFC 8628：authorization_pending 等以 HTTP 400 返回，body 仍是 JSON，不按状态码报错
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("解析飞书注册响应失败: {}", e))
}

/// 开始设备授权：返回授权链接与轮询参数。
pub async fn begin(base_url: &str) -> Result<SetupBegin, String> {
    let v = post_form(
        base_url,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;
    let res: BeginResponse = serde_json::from_value(v.clone())
        .map_err(|e| format!("begin 响应缺字段: {} ({})", e, v))?;
    let url = build_setup_url(&res.verification_uri_complete);
    Ok(SetupBegin {
        url,
        device_code: res.device_code,
        interval_secs: res.interval.unwrap_or(5),
        expires_in: res.expires_in.unwrap_or(600),
    })
}

/// 轮询一次：调用方按返回节奏继续（SlowDown 加大间隔）。
pub async fn poll(base_url: &str, device_code: &str) -> Result<SetupPoll, String> {
    let v = post_form(
        base_url,
        &[("action", "poll"), ("device_code", device_code)],
    )
    .await?;
    // 成功：含 client_id/client_secret
    if let (Some(app_id), Some(app_secret)) = (
        v.get("client_id").and_then(|x| x.as_str()),
        v.get("client_secret").and_then(|x| x.as_str()),
    ) {
        let open_id = v
            .pointer("/user_info/open_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let tenant_brand = v
            .pointer("/user_info/tenant_brand")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        return Ok(SetupPoll::Success {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            open_id,
            tenant_brand,
        });
    }
    match v.get("error").and_then(|x| x.as_str()) {
        Some("authorization_pending") => Ok(SetupPoll::Pending),
        Some("slow_down") => Ok(SetupPoll::SlowDown { interval_secs: 10 }),
        Some("access_denied") => Ok(SetupPoll::Denied),
        Some("expired_token") => Ok(SetupPoll::Expired),
        Some(other) => Err(format!("飞书注册失败: {} ({})", other, v)),
        None => Err(format!("飞书注册响应无法识别: {}", v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mock_http_server;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn build_addons_roundtrip_minimal_scopes() {
        let encoded = build_addons();
        assert!(encoded
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let gz = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&encoded)
            .unwrap();
        let mut json_str = String::new();
        GzDecoder::new(&gz[..])
            .read_to_string(&mut json_str)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(v["preset"], false);
        let scopes = v["scopes"]["tenant"].as_array().unwrap();
        assert!(scopes.iter().any(|s| s == "im:message:send_as_bot"));
        assert!(scopes.iter().any(|s| s == "im:message:update"));
        assert!(scopes.iter().any(|s| s == "im:message.p2p_msg:readonly"));
        assert_eq!(scopes.len(), 3);
        assert_eq!(v["events"]["items"]["tenant"][0], "im.message.receive_v1");
        assert_eq!(v["callbacks"]["items"][0], "card.action.trigger");
    }

    #[test]
    fn build_setup_url_contains_required_params() {
        let url = build_setup_url("https://open.feishu.cn/page/launcher?user_code=ABCD-EFGH");
        assert!(url.contains("user_code=ABCD-EFGH"));
        assert!(url.contains("from=sdk"));
        assert!(url.contains("source=agtalk"));
        assert!(url.contains("tp=sdk"));
        assert!(url.contains("createOnly=true"));
        assert!(url.contains("name=agtalk"));
        assert!(url.contains("addons="));
    }

    #[tokio::test]
    async fn begin_parses_device_code_and_url() {
        let (base, rx, _h) = mock_http_server(vec![serde_json::json!({
            "device_code": "dc-123",
            "verification_uri_complete": "https://open.feishu.cn/page/launcher?user_code=ABCD-EFGH",
            "interval": 5,
            "expires_in": 600
        })
        .to_string()]);
        let begin = begin(&base).await.unwrap();
        assert_eq!(begin.device_code, "dc-123");
        assert_eq!(begin.interval_secs, 5);
        assert_eq!(begin.expires_in, 600);
        assert!(begin.url.contains("user_code=ABCD-EFGH"));
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /oauth/v1/app/registration"));
        assert!(req.contains("action=begin"));
        assert!(req.contains("archetype=PersonalAgent"));
        assert!(req.contains("request_user_info=open_id"));
    }

    #[tokio::test]
    async fn poll_pending_then_success_with_open_id() {
        let (base, _rx, _h) = mock_http_server(vec![
            serde_json::json!({"error": "authorization_pending"}).to_string(),
            serde_json::json!({
                "client_id": "cli_xxx",
                "client_secret": "sec_yyy",
                "user_info": {"open_id": "ou_zzz", "tenant_brand": "feishu"}
            })
            .to_string(),
        ]);
        assert_eq!(poll(&base, "dc-123").await.unwrap(), SetupPoll::Pending);
        match poll(&base, "dc-123").await.unwrap() {
            SetupPoll::Success {
                app_id,
                app_secret,
                open_id,
                tenant_brand,
            } => {
                assert_eq!(app_id, "cli_xxx");
                assert_eq!(app_secret, "sec_yyy");
                assert_eq!(open_id, "ou_zzz");
                assert_eq!(tenant_brand.as_deref(), Some("feishu"));
            }
            other => panic!("expected Success, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn poll_slow_down_denied_expired() {
        let (base, _rx, _h) = mock_http_server(vec![
            serde_json::json!({"error": "slow_down"}).to_string(),
            serde_json::json!({"error": "access_denied"}).to_string(),
            serde_json::json!({"error": "expired_token"}).to_string(),
        ]);
        assert_eq!(
            poll(&base, "dc").await.unwrap(),
            SetupPoll::SlowDown { interval_secs: 10 }
        );
        assert_eq!(poll(&base, "dc").await.unwrap(), SetupPoll::Denied);
        assert_eq!(poll(&base, "dc").await.unwrap(), SetupPoll::Expired);
    }
}
