//! CLI `mem` 命名空间客户端。

use crate::cli::client::{get, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::cli::MemCmd;

pub fn dispatch(ctx: Context, cmd: MemCmd, json: bool) -> Result<(), CliError> {
    let resp = match cmd {
        MemCmd::Plan { cmd } => match cmd {
            crate::cli::MemPlanCmd::Show { target } => {
                let endpoint = if let Some(t) = target {
                    format!("/api/v1/mem/plan?target={}", t)
                } else {
                    "/api/v1/mem/plan".into()
                };
                get(&ctx, &endpoint)?
            }
            crate::cli::MemPlanCmd::Update {
                plan,
                context,
                status,
                summary,
            } => post(
                &ctx,
                "/api/v1/mem/plan",
                serde_json::json!({
                    "plan": plan,
                    "context": context,
                    "status": status,
                    "summary": summary,
                }),
            )?,
            crate::cli::MemPlanCmd::Status { target } => {
                let endpoint = if let Some(t) = target {
                    format!("/api/v1/mem/plan/status?target={}", t)
                } else {
                    "/api/v1/mem/plan/status".into()
                };
                get(&ctx, &endpoint)?
            }
        },
        MemCmd::Add {
            text,
            topic,
            ty,
            title,
            tag,
        } => post(
            &ctx,
            "/api/v1/mem/add",
            serde_json::json!({
                "text": text,
                "topic": topic.unwrap_or_else(|| "general".to_string()),
                "entry_type": ty.unwrap_or_else(|| "note".to_string()),
                "title": title,
                "tags": tag,
            }),
        )?,
        MemCmd::Search {
            query,
            topic,
            limit,
        } => {
            let mut endpoint = format!("/api/v1/mem/search?query={}", query);
            if let Some(t) = topic {
                endpoint.push_str(&format!("&topic={}", t));
            }
            if let Some(l) = limit {
                endpoint.push_str(&format!("&limit={}", l));
            }
            get(&ctx, &endpoint)?
        }
        MemCmd::Show { id } => get(&ctx, &format!("/api/v1/mem/show/{}", id))?,
        MemCmd::List { topic } => {
            let mut endpoint = "/api/v1/mem/list".to_string();
            if let Some(t) = topic {
                endpoint.push_str(&format!("?topic={}", t));
            }
            get(&ctx, &endpoint)?
        }
        MemCmd::Pack { topic, limit } => {
            let mut endpoint = "/api/v1/mem/pack".to_string();
            let mut first = true;
            if let Some(t) = topic {
                endpoint.push_str(&format!("{}topic={}", if first { "?" } else { "&" }, t));
                first = false;
            }
            if let Some(l) = limit {
                endpoint.push_str(&format!("{}limit={}", if first { "?" } else { "&" }, l));
            }
            get(&ctx, &endpoint)?
        }
    };
    print_server_msg(json, &resp);
    Ok(())
}
