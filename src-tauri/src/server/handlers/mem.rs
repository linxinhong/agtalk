//! `/api/v1/mem/*` handler。

use crate::mem::{self, collect_topics};
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::http::HeaderMap;

#[allow(clippy::result_large_err)]
fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::identity::auth::AuthenticatedSession, ServerMsg> {
    super::authenticate_req(state, headers)
}

fn is_uuid(s: &str) -> bool {
    uuid::Uuid::try_parse(s).is_ok()
}

fn status_summary_text(status: &mem::StatusSummary) -> String {
    format!("{}: {}", status.status, status.summary)
        .trim_end_matches([':', ' '])
        .to_string()
}

#[allow(clippy::result_large_err)]
fn resolve_target(state: &AppState, target: Option<&str>) -> Result<(String, String), ServerMsg> {
    if let Some(t) = target {
        if is_uuid(t) {
            let row = crate::mem::index::lookup_by_address(&state.storage, t).ok_or_else(|| {
                ServerMsg::Error {
                    code: "not_found".into(),
                    message: format!("未找到 address {} 的记忆", t),
                }
            })?;
            return Ok((row.address, row.name));
        }
        let rows = crate::mem::index::lookup_by_name(&state.storage, t);
        match rows.len() {
            0 => Err(ServerMsg::Error {
                code: "not_found".into(),
                message: format!("未找到 name {} 的记忆", t),
            }),
            1 => Ok((rows[0].address.clone(), rows[0].name.clone())),
            n => Err(ServerMsg::Error {
                code: "ambiguous".into(),
                message: format!("name {} 对应 {} 个候选，请用 address 指定", t, n),
            }),
        }
    } else {
        Err(ServerMsg::Error {
            code: "missing_target".into(),
            message: "缺少 target 参数".into(),
        })
    }
}

pub fn handle_plan_show(
    state: &AppState,
    headers: &HeaderMap,
    target: Option<String>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (target_address, target_name) = if let Some(t) = target {
        match resolve_target(state, Some(&t)) {
            Ok(pair) => pair,
            Err(e) => return e,
        }
    } else {
        (session.address, session.name)
    };

    let state_path = &session.workspace_root;
    match mem::plan_show(state_path, &target_name) {
        Ok(s) => ServerMsg::MemPlanShow {
            address: target_address,
            name: target_name,
            plan: s.plan,
            context: s.context,
            status: s.status.status,
            summary: s.status.summary,
            updated_at: s.status.updated_at,
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_plan_update(
    state: &AppState,
    headers: &HeaderMap,
    plan: Option<String>,
    context: Option<String>,
    status: Option<String>,
    summary: Option<String>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match mem::plan_update(
        &session.workspace_root,
        &session.name,
        plan,
        context,
        status,
        summary,
    ) {
        Ok(st) => {
            let topics = collect_topics(&session.workspace_root, &session.name).unwrap_or_default();
            crate::mem::index::refresh(
                &state.storage,
                &session.address,
                &status_summary_text(&st),
                &topics,
            );
            ServerMsg::MemPlanStatus {
                address: session.address,
                name: session.name,
                updated_at: st.updated_at,
                status: st.status,
                summary: st.summary,
            }
        }
        Err(e) => mem_error_msg(e),
    }
}

fn mem_error_msg(e: mem::MemError) -> ServerMsg {
    let code = match &e {
        mem::MemError::InvalidStatus(_) => "invalid_status",
        _ => "mem_error",
    };
    ServerMsg::Error {
        code: code.into(),
        message: e.to_string(),
    }
}

pub fn handle_plan_status(
    state: &AppState,
    headers: &HeaderMap,
    target: Option<String>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (target_address, target_name) = if let Some(t) = target {
        match resolve_target(state, Some(&t)) {
            Ok(pair) => pair,
            Err(e) => return e,
        }
    } else {
        (session.address, session.name)
    };

    match mem::plan_status(&session.workspace_root, &target_name) {
        Ok(st) => ServerMsg::MemPlanStatus {
            address: target_address,
            name: target_name,
            updated_at: st.updated_at,
            status: st.status,
            summary: st.summary,
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_add(
    state: &AppState,
    headers: &HeaderMap,
    text: String,
    topic: String,
    ty: String,
    title: Option<String>,
    tags: Vec<String>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match mem::add(
        &session.workspace_root,
        &session.name,
        text,
        topic,
        ty,
        title,
        tags,
    ) {
        Ok(id) => {
            let topics = collect_topics(&session.workspace_root, &session.name).unwrap_or_default();
            let status =
                mem::plan_status(&session.workspace_root, &session.name).unwrap_or_default();
            crate::mem::index::refresh(
                &state.storage,
                &session.address,
                &status_summary_text(&status),
                &topics,
            );
            ServerMsg::Ok { id }
        }
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_search(
    state: &AppState,
    headers: &HeaderMap,
    query: String,
    topic: Option<String>,
    limit: Option<usize>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match mem::search(
        &session.workspace_root,
        &session.name,
        &query,
        topic.as_deref(),
        limit,
    ) {
        Ok(entries) => ServerMsg::MemSearchResult {
            entries: entries
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect(),
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_show(state: &AppState, headers: &HeaderMap, id: String) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match mem::show_entry(&session.workspace_root, &session.name, &id) {
        Ok(entry) => ServerMsg::MemShowResult {
            entry: serde_json::to_value(entry).unwrap_or_default(),
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_list(state: &AppState, headers: &HeaderMap, topic: Option<String>) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match mem::list(&session.workspace_root, &session.name, topic.as_deref()) {
        Ok(entries) => ServerMsg::MemSearchResult {
            entries: entries
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect(),
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_pack(
    state: &AppState,
    headers: &HeaderMap,
    topic: Option<String>,
    limit: Option<usize>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let topic_str = topic.unwrap_or_default();

    match mem::pack(&session.workspace_root, &session.name, &topic_str, limit) {
        Ok(markdown) => ServerMsg::MemPack {
            topic: if topic_str.is_empty() {
                "all".to_string()
            } else {
                topic_str
            },
            markdown,
        },
        Err(e) => ServerMsg::Error {
            code: "mem_error".into(),
            message: e.to_string(),
        },
    }
}
