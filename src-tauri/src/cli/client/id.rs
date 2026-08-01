//! CLI `id` 命名空间客户端。

use crate::cli::client::{encode_query, get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::proto::{NotifyProbe, ServerMsg};

pub fn join(
    ctx: Context,
    name: Option<String>,
    intro: Option<String>,
    notify: String,
    notify_endpoint: Option<serde_json::Value>,
    notify_diagnostics: Vec<NotifyProbe>,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/join",
        serde_json::json!({
            "name": name,
            "intro": intro,
            "notify": notify,
            "notify_endpoint": notify_endpoint,
            "pid": ctx.pid,
            "start_time": ctx.start_time,
        }),
    )?;
    let mut resp = resp;
    if let ServerMsg::Identity {
        notify_diagnostics: response_diagnostics,
        ..
    } = &mut resp
    {
        *response_diagnostics = notify_diagnostics;
    }
    print_server_msg(json, &resp);
    Ok(())
}

pub fn leave(ctx: Context, purge: bool, json: bool) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/leave",
        serde_json::json!({
            "purge": purge,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn leave_by_address(
    ctx: Context,
    address: String,
    purge: bool,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/leave",
        serde_json::json!({
            "address": address,
            "purge": purge,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn prompt(ctx: Context, json: bool) -> Result<(), CliError> {
    // 读取本地 session（身份已由 Context::current 解析保证存在）
    let session = crate::identity::session_file::read(&ctx.dot_agtalk, &ctx.name)
        .map_err(|e| CliError::new("session_read_failed", e.to_string()))?;
    let p = crate::identity::prompt::onboarding_prompt(&session);
    if json {
        println!(
            "{}",
            serde_json::to_string(&p).map_err(|e| CliError::new("json_error", e.to_string()))?
        );
    } else {
        print!("{}", p.prompt_text);
    }
    Ok(())
}

pub fn show(ctx: Context, json: bool) -> Result<(), CliError> {
    let resp = get(&ctx, "/api/v1/id/me")?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn lookup(ctx: Context, name: Option<String>, json: bool) -> Result<(), CliError> {
    let endpoint = match name {
        Some(n) => encode_query("/api/v1/id/lookup", vec![("name", n)]),
        None => "/api/v1/id/lookup".into(),
    };
    let resp = get(&ctx, &endpoint)?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn cleanup(ctx: Context, execute: bool, json: bool) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/cleanup",
        serde_json::json!({ "execute": execute }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}
