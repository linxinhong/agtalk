//! CLI `tool` 命名空间客户端。

use crate::cli::client::{get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_doctor_msg, print_server_msg, CliError};
use crate::cli::ToolCmd;
use crate::config::AgConfig;
use crate::storage::Storage;
use crate::tool::DoctorContext;

pub fn dispatch(
    ctx: Option<Context>,
    cmd: ToolCmd,
    json: bool,
    as_name: Option<&str>,
) -> Result<(), CliError> {
    match cmd {
        ToolCmd::Daemon { action } => {
            let Some(ctx) = ctx else {
                return Err(CliError::new(
                    "identity_required",
                    "tool daemon 需要当前身份，请先执行 agtalk id join",
                ));
            };
            let resp = post(
                &ctx,
                "/api/v1/tool/daemon",
                serde_json::json!({ "action": action }),
            )?;
            print_server_msg(json, &resp);
            Ok(())
        }
        ToolCmd::Doctor { debug } => {
            let dot_agtalk = std::env::current_dir()
                .map_err(|e| CliError::from(e.to_string()))?
                .join(".agtalk");
            let config = AgConfig::load().map_err(|e| CliError::from(e.to_string()))?;
            // CLI 直接打开 DB 做本地检查；若 DB 被锁则降级为 None
            let storage = Storage::open().ok();
            let ctx =
                DoctorContext::new(dot_agtalk, config, storage, as_name.map(|s| s.to_string()));
            let resp = crate::tool::doctor::run(ctx);
            print_doctor_msg(json, debug, &resp);
            Ok(())
        }
        ToolCmd::Version => {
            let Some(ctx) = ctx else {
                return Err(CliError::new(
                    "identity_required",
                    "tool version 需要当前身份，请先执行 agtalk id join",
                ));
            };
            let resp = get(&ctx, "/api/v1/tool/version")?;
            print_server_msg(json, &resp);
            Ok(())
        }
        ToolCmd::Path => {
            let Some(ctx) = ctx else {
                return Err(CliError::new(
                    "identity_required",
                    "tool path 需要当前身份，请先执行 agtalk id join",
                ));
            };
            let resp = get(&ctx, "/api/v1/tool/path")?;
            print_server_msg(json, &resp);
            Ok(())
        }
    }
}
