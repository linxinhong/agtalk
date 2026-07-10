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
            MemRelationCmd::List { specialty } => {
                let list = relations::list(&ctx.dot_agtalk, &ctx.name, specialty.as_deref())
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
                specialty,
                preferred_for,
            } => {
                let tags = if tag.is_empty() { None } else { Some(tag) };
                let specialties = if specialty.is_empty() {
                    None
                } else {
                    Some(specialty)
                };
                let preferred_for = if preferred_for.is_empty() {
                    None
                } else {
                    Some(preferred_for)
                };
                let updated = relations::update(
                    &ctx.dot_agtalk,
                    &ctx.name,
                    &name_or_address,
                    relations::RelationUpdate {
                        role,
                        tags,
                        note,
                        specialties,
                        preferred_for,
                    },
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
            } => {
                // 在 CLI 边界解析文件 / stdin；daemon 只接收内容，不读取客户端路径。
                let stdin = std::io::stdin();
                let mut stdin_lock = stdin.lock();
                let mut stdin_used = false;
                let plan = plan
                    .map(|p| resolve_content_arg(&p, &mut stdin_lock, &mut stdin_used))
                    .transpose()?;
                let context = context
                    .map(|c| resolve_content_arg(&c, &mut stdin_lock, &mut stdin_used))
                    .transpose()?;
                patch(
                    &ctx,
                    "/api/v1/mem/plan",
                    serde_json::json!({
                        "plan": plan,
                        "context": context,
                        "status": status,
                        "summary": summary,
                    }),
                )?
            }
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

/// 解析 `mem plan update` 的 `--plan` / `--context` 参数：
/// - 值为 `-` 时从 `stdin` 读取；同一次命令 plan 与 context 不能同时读 stdin。
/// - 值指向已存在文件时读取文件内容。
/// - 否则按字面内容处理（保留对旧用法的兼容）。
fn resolve_content_arg<R: std::io::Read>(
    value: &str,
    stdin: &mut R,
    stdin_used: &mut bool,
) -> Result<String, CliError> {
    if value == "-" {
        if *stdin_used {
            return Err(CliError::new(
                "stdin_conflict",
                "plan 与 context 不能同时从 stdin 读取",
            ));
        }
        let mut buf = String::new();
        stdin
            .read_to_string(&mut buf)
            .map_err(|e| CliError::new("stdin_read_failed", e.to_string()))?;
        *stdin_used = true;
        return Ok(buf);
    }

    let path = std::path::Path::new(value);
    if path.is_file() {
        return std::fs::read_to_string(path).map_err(|e| {
            CliError::new(
                "plan_file_read_failed",
                format!("无法读取 {}: {}", value, e),
            )
        });
    }

    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn resolve_content_arg_reads_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plan.md");
        std::fs::write(&path, "# plan\n\n- do X").unwrap();

        let mut stdin = Cursor::new(Vec::new());
        let mut used = false;
        let resolved = resolve_content_arg(path.to_str().unwrap(), &mut stdin, &mut used).unwrap();
        assert_eq!(resolved, "# plan\n\n- do X");
        assert!(!used);
    }

    #[test]
    fn resolve_content_arg_reads_stdin_once() {
        let mut stdin = Cursor::new(b"from stdin".to_vec());
        let mut used = false;
        let resolved = resolve_content_arg("-", &mut stdin, &mut used).unwrap();
        assert_eq!(resolved, "from stdin");
        assert!(used);
    }

    #[test]
    fn resolve_content_arg_rejects_double_stdin() {
        let mut stdin = Cursor::new(b"data".to_vec());
        let mut used = false;
        resolve_content_arg("-", &mut stdin, &mut used).unwrap();
        let err = resolve_content_arg("-", &mut stdin, &mut used).unwrap_err();
        assert_eq!(err.code, "stdin_conflict");
    }

    #[test]
    fn resolve_content_arg_literal_fallback() {
        let mut stdin = Cursor::new(Vec::new());
        let mut used = false;
        let resolved = resolve_content_arg("literal markdown", &mut stdin, &mut used).unwrap();
        assert_eq!(resolved, "literal markdown");
        assert!(!used);
    }
}
