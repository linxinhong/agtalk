//! CLI `id` 命名空间客户端。

use crate::cli::client::{encode_query, get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};

pub fn join(
    ctx: Context,
    name: Option<String>,
    intro: Option<String>,
    notify: String,
    notify_endpoint: Option<serde_json::Value>,
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
