//! CLI `mem` 命名空间客户端。

use crate::cli::client::{encode_query, get, patch, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::cli::MemCmd;

pub fn dispatch(ctx: Context, cmd: MemCmd, json: bool) -> Result<(), CliError> {
    let resp = match cmd {
        MemCmd::Plan { cmd } => match cmd {
            crate::cli::MemPlanCmd::Show { target } => {
                let params = target.map(|t| vec![("target", t)]).unwrap_or_default();
                get(&ctx, &encode_query("/api/v1/mem/plan", params))?
            }
            crate::cli::MemPlanCmd::Update {
                plan,
                context,
                status,
                summary,
            } => patch(
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
                let params = target.map(|t| vec![("target", t)]).unwrap_or_default();
                get(&ctx, &encode_query("/api/v1/mem/plan/status", params))?
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
            let mut params = vec![("query", query)];
            if let Some(t) = topic {
                params.push(("topic", t));
            }
            if let Some(l) = limit {
                params.push(("limit", l.to_string()));
            }
            get(&ctx, &encode_query("/api/v1/mem/search", params))?
        }
        MemCmd::Show { id } => get(&ctx, &format!("/api/v1/mem/show/{}", id))?,
        MemCmd::List { topic } => {
            let params = topic.map(|t| vec![("topic", t)]).unwrap_or_default();
            get(&ctx, &encode_query("/api/v1/mem/list", params))?
        }
        MemCmd::Pack {
            topic_pos,
            topic,
            limit,
        } => {
            let topic = topic.or(topic_pos);
            let mut params = Vec::new();
            if let Some(t) = topic {
                params.push(("topic", t));
            }
            if let Some(l) = limit {
                params.push(("limit", l.to_string()));
            }
            get(&ctx, &encode_query("/api/v1/mem/pack", params))?
        }
    };
    print_server_msg(json, &resp);
    Ok(())
}
