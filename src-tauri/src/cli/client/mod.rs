//! CLI HTTP 客户端共享工具。

use crate::cli::context::Context;
use crate::cli::output::CliError;
use crate::proto::ServerMsg;
use reqwest::blocking::Client;
use serde::Serialize;

pub mod config;
pub mod id;
pub mod mem;
pub mod msg;
pub mod run;
pub mod tool;

pub fn post(ctx: &Context, endpoint: &str, body: impl Serialize) -> Result<ServerMsg, CliError> {
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| CliError::new("client", e.to_string()))?;
    let url = format!("{}{}", ctx.base_url, endpoint);
    let resp = client
        .post(&url)
        .header("X-AgTalk-Address", ctx.address.clone())
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string())
        .json(&body)
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

pub fn get(ctx: &Context, endpoint: &str) -> Result<ServerMsg, CliError> {
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| CliError::new("client", e.to_string()))?;
    let url = format!("{}{}", ctx.base_url, endpoint);
    let resp = client
        .get(&url)
        .header("X-AgTalk-Address", ctx.address.clone())
        .header("X-AgTalk-Pid", ctx.pid.to_string())
        .header("X-AgTalk-Start-Time", ctx.start_time.to_string())
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
