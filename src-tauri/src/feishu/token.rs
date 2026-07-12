//! `tenant_access_token` 进程内缓存：过期前 60 秒余量刷新。
//! 全链路只用 tenant token（应用身份），不做 user token。

use super::FeishuError;
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TOKEN_URI: &str = "/auth/v3/tenant_access_token/internal";
/// 过期余量：提前 60 秒刷新，避免边界失效。
const EXPIRE_MARGIN: Duration = Duration::from_secs(60);

struct Cached {
    token: String,
    expires_at: Instant,
}

pub struct TokenCache {
    http: reqwest::Client,
    base_url: String,
    app_id: String,
    app_secret: String,
    cached: Mutex<Option<Cached>>,
}

impl TokenCache {
    pub fn new(base_url: &str, app_id: &str, app_secret: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            cached: Mutex::new(None),
        }
    }

    /// 获取可用 token：命中缓存直接返回，否则请求飞书并缓存。
    pub async fn tenant_token(&self) -> Result<String, FeishuError> {
        {
            let guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = guard.as_ref() {
                if Instant::now() + EXPIRE_MARGIN < c.expires_at {
                    return Ok(c.token.clone());
                }
            }
        }
        let url = format!("{}{}", self.base_url, TOKEN_URI);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await?;
        let v: Value = resp.json().await?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            return Err(FeishuError::Api {
                code,
                message: v
                    .get("msg")
                    .and_then(|m| m.as_str())
                    .unwrap_or("tenant_access_token failed")
                    .to_string(),
            });
        }
        let token = v
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| FeishuError::Proto("missing tenant_access_token".into()))?
            .to_string();
        let expire_secs = v.get("expire").and_then(|e| e.as_u64()).unwrap_or(7200);
        let mut guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Cached {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(expire_secs),
        });
        Ok(token)
    }

    /// 测试辅助：直接注入缓存项。
    #[cfg(test)]
    fn inject_cache(&self, token: &str, ttl: Duration) {
        let mut guard = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(Cached {
            token: token.to_string(),
            expires_at: Instant::now() + ttl,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mock_http_server;

    fn token_response(token: &str, expire: u64) -> String {
        serde_json::json!({
            "code": 0,
            "msg": "ok",
            "tenant_access_token": token,
            "expire": expire,
        })
        .to_string()
    }

    #[tokio::test]
    async fn first_fetch_then_cache_hit() {
        let (base, rx, handle) = mock_http_server(vec![token_response("t-first", 7200)]);
        let cache = TokenCache::new(&base, "cli_a", "secret");
        assert_eq!(cache.tenant_token().await.unwrap(), "t-first");
        // 第二次命中缓存，不再请求（mock 只预备了一个响应）
        assert_eq!(cache.tenant_token().await.unwrap(), "t-first");
        assert!(rx
            .recv()
            .unwrap()
            .starts_with("POST /auth/v3/tenant_access_token/internal"));
        assert!(rx.try_recv().is_err(), "缓存命中不应有第二次请求");
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn expired_cache_refreshes() {
        let (base, _rx, handle) = mock_http_server(vec![token_response("t-fresh", 7200)]);
        let cache = TokenCache::new(&base, "cli_a", "secret");
        // 注入一个 10 秒后过期的缓存（小于 60 秒余量，视为过期）
        cache.inject_cache("t-stale", Duration::from_secs(10));
        assert_eq!(cache.tenant_token().await.unwrap(), "t-fresh");
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn api_error_propagates_code() {
        let err = serde_json::json!({ "code": 99991672, "msg": "app not found" }).to_string();
        let (base, _rx, handle) = mock_http_server(vec![err]);
        let cache = TokenCache::new(&base, "bad", "bad");
        let e = cache.tenant_token().await.unwrap_err();
        assert!(e.to_string().contains("99991672"), "{}", e);
        handle.join().unwrap();
    }
}
