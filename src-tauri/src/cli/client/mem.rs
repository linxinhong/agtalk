//! CLI `mem` 命名空间客户端。

use crate::cli::client::{encode_query, get, patch, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::cli::{MemCmd, MemRelationCmd};
use crate::identity::relations;
use crate::proto::ServerMsg;

pub fn dispatch(ctx: Context, cmd: MemCmd, json: bool) -> Result<(), CliError> {
    let resp = match cmd {
        MemCmd::Relation { cmd } => match cmd {
            MemRelationCmd::List => {
                let list = relations::list(&ctx.dot_agtalk, &ctx.name)
                    .map_err(|e| CliError::new("relation_list_failed", e.to_string()))?;
                ServerMsg::MemRelationList { relations: list }
            }
            MemRelationCmd::Show { name_or_address } => {
                let found = relations::find(&ctx.dot_agtalk, &ctx.name, &name_or_address)
                    .map_err(|e| CliError::new("relation_find_failed", e.to_string()))?;
                match found {
                    Some(relation) => ServerMsg::MemRelation { relation },
                    None => ServerMsg::Error {
                        code: "relation_not_found".into(),
                        message: format!("未找到 peer: {}", name_or_address),
                    },
                }
            }
            MemRelationCmd::Update {
                name_or_address,
                role,
                tag,
                note,
            } => {
                let tags = if tag.is_empty() { None } else { Some(tag) };
                let updated = relations::update(
                    &ctx.dot_agtalk,
                    &ctx.name,
                    &name_or_address,
                    role,
                    tags,
                    note,
                )
                .map_err(|e| CliError::new("relation_update_failed", e.to_string()))?;
                match updated {
                    Some(relation) => ServerMsg::MemRelation { relation },
                    None => ServerMsg::Error {
                        code: "relation_not_found".into(),
                        message: format!("未找到 peer: {}", name_or_address),
                    },
                }
            }
        },
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
