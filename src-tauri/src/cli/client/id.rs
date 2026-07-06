//! CLI `id` 命名空间客户端。

use crate::cli::client::{get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};

pub fn join(
    ctx: Context,
    name: Option<String>,
    intro: Option<String>,
    workspace: Option<String>,
    notify: String,
    json: bool,
) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/join",
        serde_json::json!({
            "name": name,
            "intro": intro,
            "workspace": workspace,
            "notify": notify,
            "pid": ctx.pid,
            "start_time": ctx.start_time,
        }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}

pub fn leave(ctx: Context, _purge: bool, json: bool) -> Result<(), CliError> {
    let resp = post(
        &ctx,
        "/api/v1/id/leave",
        serde_json::json!({
            "purge": false,
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
    let endpoint = if let Some(ref n) = name {
        // 简单 name 通常不含需编码字符；复杂 name 建议后续引入 urlencoding
        format!("/api/v1/id/lookup?name={}", n)
    } else {
        "/api/v1/id/lookup".into()
    };
    let resp = get(&ctx, &endpoint)?;
    print_server_msg(json, &resp);
    Ok(())
}
