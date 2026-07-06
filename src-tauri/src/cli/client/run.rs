//! CLI `run` 客户端。

use crate::cli::context::Context;
use crate::cli::output::CliError;
use crate::cli::runner;
use std::path::PathBuf;

pub fn run(ctx: Option<Context>, file: Option<PathBuf>, json: bool) -> Result<(), CliError> {
    let ctx = ctx.ok_or_else(|| {
        CliError::new(
            "identity_required",
            "run 命令需要当前身份，请先执行 agtalk id join",
        )
    })?;

    runner::run(ctx, file, json)
}
