//! CLI `config` 命名空间客户端。

use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::cli::ConfigCmd;
use crate::config::AgConfig;
use crate::proto::ServerMsg;
use reqwest::blocking::{Client, RequestBuilder};

pub fn dispatch(ctx: Option<Context>, cmd: ConfigCmd, json: bool) -> Result<(), CliError> {
    let resp = match cmd {
        ConfigCmd::Show => request(ctx.as_ref(), reqwest::Method::GET, "/api/v1/config", None)?,
        ConfigCmd::Get { key } => request(
            ctx.as_ref(),
            reqwest::Method::GET,
            &format!("/api/v1/config/{}", key),
            None,
        )?,
        ConfigCmd::Set { key, value } => request(
            ctx.as_ref(),
            reqwest::Method::PATCH,
            &format!("/api/v1/config/{}", key),
            Some(serde_json::json!({ "value": value })),
        )?,
        ConfigCmd::Path => request(
            ctx.as_ref(),
            reqwest::Method::GET,
            "/api/v1/config/path",
            None,
        )?,
    };
    print_server_msg(json, &resp);
    Ok(())
}

fn base_url(ctx: Option<&Context>) -> Result<String, CliError> {
    if let Some(ctx) = ctx {
        return Ok(ctx.base_url.clone());
    }
    let cfg = AgConfig::load().map_err(|e| CliError::new("config", e.to_string()))?;
    Ok(format!("http://127.0.0.1:{}", cfg.http_port))
}

fn request(
    ctx: Option<&Context>,
    method: reqwest::Method,
    endpoint: &str,
    body: Option<serde_json::Value>,
) -> Result<ServerMsg, CliError> {
    let base = base_url(ctx)?;
    let url = format!("{}{}", base, endpoint);
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| CliError::new("client", e.to_string()))?;

    let mut builder: RequestBuilder = client.request(method, &url);
    if let Some(c) = ctx {
        builder = builder
            .header("X-AgTalk-Address", c.address.clone())
            .header("X-AgTalk-Pid", c.pid.to_string())
            .header("X-AgTalk-Start-Time", c.start_time.to_string());
    }
    if let Some(b) = body {
        builder = builder.json(&b);
    }

    let resp = builder
        .send()
        .map_err(|e| CliError::new("http", e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| CliError::new("http", e.to_string()))?;
    if !status.is_success() {
        if let Ok(ServerMsg::Error { code, message }) = serde_json::from_str(&text) {
            return Err(CliError::new(code, message));
        }
        return Err(CliError::new("http", format!("HTTP {}: {}", status, text)));
    }
    serde_json::from_str(&text).map_err(|e| CliError::new("parse", format!("解析响应失败: {}", e)))
}
