//! CLI HTTP 客户端共享工具。

use crate::cli::context::Context;
use crate::cli::output::CliError;
use crate::proto::ServerMsg;
use reqwest::blocking::Client;
use serde::Serialize;

pub mod config;
pub mod graph;
pub mod id;
pub mod mem;
pub mod msg;
pub mod run;
pub mod tool;

pub fn post(ctx: &Context, endpoint: &str, body: impl Serialize) -> Result<ServerMsg, CliError> {
    request(
        ctx,
        reqwest::Method::POST,
        endpoint,
        Some(serde_json::to_value(body).map_err(|e| CliError::new("parse", e.to_string()))?),
    )
}

pub fn get(ctx: &Context, endpoint: &str) -> Result<ServerMsg, CliError> {
    request(ctx, reqwest::Method::GET, endpoint, None)
}

pub fn patch(ctx: &Context, endpoint: &str, body: impl Serialize) -> Result<ServerMsg, CliError> {
    request(
        ctx,
        reqwest::Method::PATCH,
        endpoint,
        Some(serde_json::to_value(body).map_err(|e| CliError::new("parse", e.to_string()))?),
    )
}

/// 构建带 URL 编码的 query endpoint。
pub fn encode_query(path: &str, params: Vec<(&str, String)>) -> String {
    let base = "http://127.0.0.1:1";
    let mut url = reqwest::Url::parse(base).expect("static base URL");
    url.set_path(path);
    {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in params {
            pairs.append_pair(k, &v);
        }
    }
    match url.query() {
        Some(q) if !q.is_empty() => format!("{}?{}", path, q),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_query_without_params() {
        assert_eq!(encode_query("/api/v1/id/show", vec![]), "/api/v1/id/show");
    }

    #[test]
    fn encode_query_with_spaces_and_special_chars() {
        let endpoint = encode_query(
            "/api/v1/id/lookup",
            vec![("name", "hello world".to_string())],
        );
        assert_eq!(endpoint, "/api/v1/id/lookup?name=hello+world");

        let endpoint = encode_query("/api/v1/id/lookup", vec![("name", "a&b=c?d".to_string())]);
        assert_eq!(endpoint, "/api/v1/id/lookup?name=a%26b%3Dc%3Fd");
    }

    #[test]
    fn encode_query_multiple_params() {
        let endpoint = encode_query(
            "/api/v1/mem/list",
            vec![
                ("topic", "agent learning".to_string()),
                ("limit", "10".to_string()),
            ],
        );
        assert_eq!(endpoint, "/api/v1/mem/list?topic=agent+learning&limit=10");
    }
}

fn request(
    ctx: &Context,
    method: reqwest::Method,
    endpoint: &str,
    body: Option<serde_json::Value>,
) -> Result<ServerMsg, CliError> {
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| CliError::new("client", e.to_string()))?;
    let url = format!("{}{}", ctx.base_url, endpoint);
    let mut builder = client
        .request(method, &url)
        .header("X-AgTalk-Address", ctx.address.clone())
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string())
        .header(
            "X-AgTalk-Workspace-Root",
            ctx.dot_agtalk.to_string_lossy().as_ref(),
        );
    if let Some(b) = body {
        builder = builder.json(&b);
    }

    let resp = builder
        .send()
        .map_err(|e| CliError::new("http", e.to_string()))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| CliError::new("http", e.to_string()))?;
    if !status.is_success() {
        if let Ok(ServerMsg::Error { code, message }) = serde_json::from_str(&body) {
            return Err(CliError::new(code, message));
        }
        return Err(CliError::new("http", format!("HTTP {}: {}", status, body)));
    }
    serde_json::from_str(&body).map_err(|e| CliError::new("parse", format!("解析响应失败: {}", e)))
}
