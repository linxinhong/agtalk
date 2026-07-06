//! CLI `run` 客户端。

use crate::cli::client::post;
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use std::path::PathBuf;

pub fn run(ctx: Option<Context>, file: Option<PathBuf>, json: bool) -> Result<(), CliError> {
    let ctx = ctx.ok_or_else(|| {
        CliError::new(
            "identity_required",
            "run 命令需要当前身份，请先执行 agtalk id join",
        )
    })?;

    let file = match file {
        Some(p) => p,
        None => ctx
            .dot_agtalk
            .join("runs")
            .join(format!("{}.yaml", ctx.name)),
    };

    let resp = post(
        &ctx,
        "/api/v1/run",
        serde_json::json!({ "file": file.to_string_lossy() }),
    )?;
    print_server_msg(json, &resp);
    Ok(())
}
