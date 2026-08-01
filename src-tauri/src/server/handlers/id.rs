//! `/api/v1/id/*` handler。

use crate::identity::agents_map;
use crate::identity::mailbox as mailbox_db;
use crate::identity::session_file::{NotifyTarget, SessionFile, SessionNotify};
use crate::identity::{browser_session, session_file as session_file_mod};
use crate::notify;
use crate::proto::ServerMsg;
use crate::server::handlers::{auth_error, browser_token};
use crate::server::state::AppState;
use axum::http::HeaderMap;
use sysinfo::{Pid, System};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub fn handle_join(
    state: &AppState,
    workspace_root: &std::path::Path,
    name: Option<String>,
    intro: Option<String>,
    notify: String,
    notify_endpoint: Option<serde_json::Value>,
    pid: u32,
    start_time: u64,
) -> ServerMsg {
    if let Err(e) = validate_pid(pid, start_time) {
        return auth_error(e);
    }

    let name = name.unwrap_or_else(|| format!("agent-{}", short_id()));
    let existing_session = session_file_mod::read(workspace_root, &name).ok();
    let workspace_root_str = workspace_root.to_string_lossy().into_owned();

    let (address, final_intro, notify_channel, notify_target) =
        if let Some(ref session) = existing_session {
            let final_intro = intro.unwrap_or_else(|| session.intro.clone());

            let (notify_channel, notify_target) =
                match resolve_notify(&notify, notify_endpoint.clone()) {
                    Ok(v) => v,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: e,
                        }
                    }
                };

            if let Err(e) = mailbox_db::revive_with_notify(
                &state.storage,
                &session.address,
                &name,
                &final_intro,
                "",
                &notify_channel,
                &notify_target_json(&notify_target),
                &workspace_root_str,
            ) {
                return ServerMsg::Error {
                    code: "join_failed".into(),
                    message: e.to_string(),
                };
            }

            (
                session.address.clone(),
                final_intro,
                notify_channel,
                notify_target,
            )
        } else {
            let final_intro = intro.unwrap_or_default();
            let (notify_channel, notify_target) =
                match resolve_notify(&notify, notify_endpoint.clone()) {
                    Ok(v) => v,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: e,
                        }
                    }
                };
            let address = match mailbox_db::create_with_notify(
                &state.storage,
                &name,
                &final_intro,
                "",
                &notify_channel,
                &notify_target_json(&notify_target),
                &workspace_root_str,
            ) {
                Ok(addr) => addr,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "join_failed".into(),
                        message: e.to_string(),
                    }
                }
            };

            (address, final_intro, notify_channel, notify_target)
        };

    let registered_by = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .ok();

    let notify = SessionNotify {
        channel: notify_channel.clone(),
        endpoint: match &notify_target {
            NotifyTarget::None => serde_json::Value::Null,
            NotifyTarget::Plugin { endpoint, .. } => endpoint.clone(),
        },
    };
    let notify_ready = !notify.channel.eq_ignore_ascii_case("none") && !notify.channel.is_empty();

    let session = SessionFile {
        version: 2,
        address: address.clone(),
        name: name.clone(),
        intro: final_intro,
        created_at: existing_session
            .map(|s| s.created_at)
            .unwrap_or_else(iso_now),
        registered_by,
        notify,
    };

    if let Err(e) = session_file_mod::write(workspace_root, &name, &session) {
        return ServerMsg::Error {
            code: "session_write_failed".into(),
            message: e.to_string(),
        };
    }
    // join 前清理同 name 的 dead anchors，保留 live PID 复用能力。
    let _ = agents_map::cleanup_dead_anchors_for_name(workspace_root, &name);
    if let Err(e) = agents_map::register_pid(workspace_root, pid, &name, start_time) {
        return ServerMsg::Error {
            code: "agents_map_failed".into(),
            message: e.to_string(),
        };
    }

    let memory_path = workspace_root.join(&name).join("memory");
    crate::mem::index::register(&state.storage, &address, &name, "", &memory_path);

    let notify_channel = session.notify_channel();
    ServerMsg::Identity {
        address,
        name,
        intro: session.intro,
        notify_channel,
        notify_ready,
        workspace_root: workspace_root_str,
        notify_diagnostics: Vec::new(),
    }
}

pub fn handle_show(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let session = match super::authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (intro, notify_channel) = session_file_mod::read(&session.workspace_root, &session.name)
        .map(|s| (s.intro.clone(), s.notify_channel()))
        .unwrap_or_default();
    let notify_ready = !notify_channel.eq_ignore_ascii_case("none") && !notify_channel.is_empty();
    ServerMsg::Identity {
        address: session.address,
        name: session.name,
        intro,
        notify_channel,
        notify_ready,
        workspace_root: session.workspace_root.to_string_lossy().into_owned(),
        notify_diagnostics: Vec::new(),
    }
}

pub fn handle_lookup(state: &AppState, name: Option<String>) -> ServerMsg {
    match crate::routing::lookup::lookup(&state.storage, name.as_deref()) {
        Ok(mbs) => {
            let mailboxes = mbs.iter().map(build_lookup_mailbox).collect();
            ServerMsg::LookupResult { mailboxes }
        }
        Err(e) => ServerMsg::Error {
            code: "lookup_failed".into(),
            message: e.to_string(),
        },
    }
}

pub(crate) fn build_lookup_mailbox(
    mb: &crate::identity::mailbox::Mailbox,
) -> crate::proto::LookupMailbox {
    let channel = mb.notify_channel.clone();
    let notify = notify_summary(&channel);
    let ready = !channel.eq_ignore_ascii_case("none") && !channel.is_empty();
    crate::proto::LookupMailbox::from_mailbox(mb, channel, notify, ready)
}

fn notify_summary(channel: &str) -> String {
    if channel.eq_ignore_ascii_case("none") || channel.is_empty() {
        return "none".to_string();
    }
    if channel.eq_ignore_ascii_case("zellij") || channel.eq_ignore_ascii_case("tmux") {
        return channel.to_lowercase();
    }
    if channel.starts_with("plugin:") {
        return channel.to_string();
    }
    "unknown".to_string()
}

pub fn handle_leave(
    state: &AppState,
    headers: &HeaderMap,
    address_override: Option<String>,
    purge: bool,
) -> ServerMsg {
    let (address, name, workspace_root) = match address_override {
        Some(addr) => {
            let mb = match mailbox_db::get_by_address(&state.storage, &addr) {
                Ok(Some(mb)) => mb,
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "leave_failed".into(),
                        message: format!("address 不存在或已离开: {}", addr),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "leave_failed".into(),
                        message: e.to_string(),
                    }
                }
            };
            (addr, mb.name, state.dot_agtalk.clone())
        }
        None => {
            let session = match super::authenticate_req(state, headers) {
                Ok(s) => s,
                Err(e) => return e,
            };
            (
                session.address,
                session.name,
                session.workspace_root.clone(),
            )
        }
    };

    if let Err(e) = mailbox_db::mark_left(&state.storage, &address) {
        return ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        };
    }

    let removed_session = if purge {
        session_file_mod::remove(&workspace_root, &name).is_ok()
    } else {
        false
    };

    // 清理所有指向该 name 的 pid 锚点，避免 stale agents.json。
    let _ = agents_map::remove_by_name(&workspace_root, &name);
    crate::mem::index::remove(&state.storage, &address);

    ServerMsg::IdentityLeft {
        address,
        name,
        removed_session,
        purge,
    }
}

/// 批量清理无效/未激活身份。
/// 默认 dry-run，返回候选列表；`execute=true` 时执行删除。
pub fn handle_cleanup(
    state: &AppState,
    workspace_root: &std::path::Path,
    execute: bool,
) -> ServerMsg {
    let mut removed: Vec<crate::proto::CleanupItem> = Vec::new();
    let skipped: Vec<crate::proto::CleanupItem> = Vec::new();

    // 1. 收集 DB 中所有 mailbox。
    let mailboxes = match mailbox_db::list_including_left(&state.storage) {
        Ok(mbs) => mbs,
        Err(e) => {
            return ServerMsg::Error {
                code: "cleanup_failed".into(),
                message: e.to_string(),
            }
        }
    };
    let mb_by_address: std::collections::HashMap<String, &mailbox_db::Mailbox> = mailboxes
        .iter()
        .map(|mb| (mb.address.clone(), mb))
        .collect();
    let _mb_by_name: std::collections::HashMap<String, &mailbox_db::Mailbox> =
        mailboxes.iter().map(|mb| (mb.name.clone(), mb)).collect();

    // 2. 收集文件系统中所有 session。
    let session_names = match session_file_mod::list_session_names(workspace_root) {
        Ok(names) => names,
        Err(e) => {
            return ServerMsg::Error {
                code: "cleanup_failed".into(),
                message: e.to_string(),
            }
        }
    };
    let mut session_by_name: std::collections::HashMap<String, session_file_mod::SessionFile> =
        std::collections::HashMap::new();
    for name in &session_names {
        if let Ok(session) = session_file_mod::read(workspace_root, name) {
            session_by_name.insert(name.clone(), session);
        }
    }
    // 3. stale mailbox：DB 有活跃记录，但 session 缺失或 address 不匹配。
    // 已 left 的 mailbox 跳过；它要么对应 stale_session，要么是正常历史保留。
    for mb in &mailboxes {
        if mb.left_at.is_some() {
            continue;
        }
        let stale = !matches!(
            session_by_name.get(&mb.name),
            Some(session) if session.address == mb.address
        );
        if stale {
            let item = crate::proto::CleanupItem {
                name: mb.name.clone(),
                address: mb.address.clone(),
                reason: "stale_mailbox".into(),
            };
            if execute {
                // 统一标记 left，避免外键约束导致物理删除失败；同时清理 mem_index。
                let _ = mailbox_db::mark_left(&state.storage, &mb.address);
                crate::mem::index::remove(&state.storage, &mb.address);
                removed.push(item);
            } else {
                removed.push(item);
            }
        }
    }

    // 4. stale session：session 存在，但 DB mailbox 缺失或已 left。
    for (name, session) in &session_by_name {
        let stale = !matches!(
            mb_by_address.get(&session.address),
            Some(mb) if mb.left_at.is_none() && mb.name == *name
        );
        if stale {
            let item = crate::proto::CleanupItem {
                name: name.clone(),
                address: session.address.clone(),
                reason: "stale_session".into(),
            };
            if execute {
                let _ = session_file_mod::remove(workspace_root, name);
                removed.push(item);
            } else {
                removed.push(item);
            }
        }
    }

    // 5. stale pid anchor：agents.json 指向的 session 已不存在。
    let valid_session_names: Vec<String> = session_by_name.keys().cloned().collect();
    match agents_map::cleanup_stale_anchors(workspace_root, &valid_session_names) {
        Ok(stale_pids) => {
            for (pid, name) in stale_pids {
                let address = session_by_name
                    .get(&name)
                    .map(|s| s.address.clone())
                    .unwrap_or_default();
                removed.push(crate::proto::CleanupItem {
                    name: format!("{} (pid {})", name, pid),
                    address,
                    reason: "stale_pid_anchor".into(),
                });
            }
        }
        Err(e) => {
            return ServerMsg::Error {
                code: "cleanup_failed".into(),
                message: e.to_string(),
            }
        }
    }

    // 6. dead pid anchor：进程已不存在或 start_time 不匹配。
    if execute {
        match agents_map::cleanup_dead_anchors(workspace_root) {
            Ok(dead_pids) => {
                for (pid, name) in dead_pids {
                    let address = session_by_name
                        .get(&name)
                        .map(|s| s.address.clone())
                        .unwrap_or_default();
                    removed.push(crate::proto::CleanupItem {
                        name: format!("{} (pid {})", name, pid),
                        address,
                        reason: "dead_pid_anchor".into(),
                    });
                }
            }
            Err(e) => {
                return ServerMsg::Error {
                    code: "cleanup_failed".into(),
                    message: e.to_string(),
                }
            }
        }
    }

    if !execute {
        ServerMsg::CleanupResult {
            dry_run: true,
            removed,
            skipped,
        }
    } else {
        ServerMsg::CleanupResult {
            dry_run: false,
            removed,
            skipped,
        }
    }
}

pub fn handle_browser_join(
    state: &AppState,
    name: Option<String>,
    intro: Option<String>,
    workspace: Option<String>,
) -> ServerMsg {
    match browser_session::create(&state.storage, name, intro, workspace) {
        Ok((address, name, token)) => ServerMsg::BrowserJoinResult {
            address,
            name,
            token,
        },
        Err(e) => ServerMsg::Error {
            code: "join_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_browser_leave(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let Some(token) = browser_token(headers) else {
        return auth_error("缺少 X-AgTalk-Browser-Token".into());
    };
    match browser_session::delete(&state.storage, &token) {
        Ok(()) => ServerMsg::Pong,
        Err(e) => ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        },
    }
}

fn validate_pid(pid: u32, start_time: u64) -> Result<(), String> {
    if start_time == 0 {
        return Err("start_time 不可能为 0（进程可能已退出或 sysinfo 读不到）".into());
    }
    let mut sys = System::new_all();
    sys.refresh_processes();
    let process = sys.process(Pid::from(pid as usize)).ok_or("进程不存在")?;
    let os_start = process.start_time();
    if os_start == 0 {
        return Err("OS 返回 start_time=0，进程状态异常".into());
    }
    if os_start != start_time {
        return Err("PID start_time 不匹配（可能已被 OS 复用）".into());
    }
    Ok(())
}

/// 统一解析 notify 参数：auto/none/plugin:<name>。
/// 优先使用 CLI 侧 discover 传来的 `notify_endpoint`；没有时 daemon 侧兜底 discover。
/// 显式 plugin:<name>  discover 失败时返回错误，不静默降级。
fn resolve_notify(
    notify: &str,
    notify_endpoint: Option<serde_json::Value>,
) -> Result<(String, NotifyTarget), String> {
    if notify.eq_ignore_ascii_case("none") {
        return Ok(("none".to_string(), NotifyTarget::None));
    }
    if let Some(plugin_name) = notify.strip_prefix("plugin:") {
        // 优先 CLI 传来的 endpoint。
        if let Some(endpoint) = notify_endpoint {
            return Ok((
                notify.to_string(),
                NotifyTarget::Plugin {
                    name: plugin_name.to_string(),
                    endpoint,
                },
            ));
        }
        // 否则尝试 daemon 侧 discover（向后兼容无 CLI discover 的调用方）。
        if let Some(channel) = notify::channel_from_name(notify) {
            match channel.discover() {
                Ok(Some(endpoint)) if endpoint.ready => {
                    return Ok((
                        notify.to_string(),
                        NotifyTarget::Plugin {
                            name: plugin_name.to_string(),
                            endpoint: endpoint.endpoint,
                        },
                    ));
                }
                Ok(Some(endpoint)) => {
                    return Err(format!(
                        "plugin:{} discover 未就绪: {}",
                        plugin_name, endpoint.message
                    ));
                }
                Ok(None) => {
                    return Err(format!("plugin:{} discover 返回空", plugin_name));
                }
                Err(e) => {
                    return Err(format!("plugin:{} discover 失败: {}", plugin_name, e));
                }
            }
        }
        return Err(format!("plugin:{} 通道无效", plugin_name));
    }
    if notify.eq_ignore_ascii_case("auto") {
        return Ok(notify::auto_detect());
    }
    // 未知通道统一降级为 none。
    tracing::warn!("未知 notify 通道 '{}', 降级为 none", notify);
    Ok(("none".to_string(), NotifyTarget::None))
}

fn short_id() -> String {
    Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("")
        .to_string()
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn notify_target_json(target: &NotifyTarget) -> serde_json::Value {
    serde_json::to_value(target).unwrap_or(serde_json::Value::Null)
}
#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
