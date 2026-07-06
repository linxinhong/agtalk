//! `/api/v1/id/*` handler。

use crate::identity::agents_map;
use crate::identity::mailbox as mailbox_db;
use crate::identity::session_file::{NotifyTarget, SessionFile};
use crate::identity::{browser_session, session_file as session_file_mod};
use crate::notify;
use crate::proto::ServerMsg;
use crate::server::handlers::{auth_error, browser_token};
use crate::server::state::AppState;
use axum::http::HeaderMap;
use sysinfo::{Pid, System};
use uuid::Uuid;

pub fn handle_join(
    state: &AppState,
    name: Option<String>,
    intro: Option<String>,
    workspace: Option<String>,
    notify: String,
    pid: u32,
    start_time: u64,
) -> ServerMsg {
    if let Err(e) = validate_pid(pid, start_time) {
        return auth_error(e);
    }

    let name = name.unwrap_or_else(|| format!("agent-{}", short_id()));
    let existing_session = session_file_mod::read(&state.dot_agtalk, &name).ok();

    let (address, final_intro, final_workspace, notify_channel, notify_target) =
        if let Some(ref session) = existing_session {
            let final_intro = intro.unwrap_or_else(|| session.intro.clone());
            let final_workspace = workspace.unwrap_or_else(|| session.workspace.clone());

            if let Err(e) = mailbox_db::revive(
                &state.storage,
                &session.address,
                &name,
                &final_intro,
                &final_workspace,
            ) {
                return ServerMsg::Error {
                    code: "join_failed".into(),
                    message: e.to_string(),
                };
            }

            let (notify_channel, notify_target) = if notify.eq_ignore_ascii_case("auto") {
                if session.notify_channel.is_empty()
                    || session.notify_channel.eq_ignore_ascii_case("auto")
                {
                    notify::auto_detect()
                } else {
                    (
                        session.notify_channel.clone(),
                        session.notify_target.clone(),
                    )
                }
            } else if notify.eq_ignore_ascii_case("none") {
                ("none".to_string(), NotifyTarget::None)
            } else {
                let (_, _, target) = notify::resolve_channel(&notify);
                (notify, target)
            };

            (
                session.address.clone(),
                final_intro,
                final_workspace,
                notify_channel,
                notify_target,
            )
        } else {
            let final_intro = intro.unwrap_or_default();
            let final_workspace = workspace.unwrap_or_default();
            let address =
                match mailbox_db::create(&state.storage, &name, &final_intro, &final_workspace) {
                    Ok(addr) => addr,
                    Err(e) => {
                        return ServerMsg::Error {
                            code: "join_failed".into(),
                            message: e.to_string(),
                        }
                    }
                };

            let (notify_channel, notify_target) = if notify.eq_ignore_ascii_case("auto") {
                notify::auto_detect()
            } else if notify.eq_ignore_ascii_case("none") {
                ("none".to_string(), NotifyTarget::None)
            } else {
                let (_, _, target) = notify::resolve_channel(&notify);
                (notify, target)
            };

            (
                address,
                final_intro,
                final_workspace,
                notify_channel,
                notify_target,
            )
        };

    let command = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "agtalk".to_string());

    let session = SessionFile {
        address: address.clone(),
        name: name.clone(),
        workspace: final_workspace,
        intro: final_intro,
        created_at: existing_session
            .map(|s| s.created_at)
            .unwrap_or_else(iso_now),
        command,
        notify_channel,
        notify_target,
    };

    if let Err(e) = session_file_mod::write(&state.dot_agtalk, &name, &session) {
        return ServerMsg::Error {
            code: "session_write_failed".into(),
            message: e.to_string(),
        };
    }
    if let Err(e) = agents_map::register_pid(&state.dot_agtalk, pid, &name, start_time) {
        return ServerMsg::Error {
            code: "agents_map_failed".into(),
            message: e.to_string(),
        };
    }

    let memory_path = state.dot_agtalk.join(&name).join("memory");
    crate::mem::index::register(
        &state.storage,
        &address,
        &name,
        &session.workspace,
        &memory_path,
    );

    ServerMsg::Ok { id: address }
}

pub fn handle_show(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let session = match super::authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    ServerMsg::Identity {
        address: session.address,
        name: session.name,
        workspace: session.workspace,
        intro: "".into(),
    }
}

pub fn handle_lookup(state: &AppState, name: Option<String>) -> ServerMsg {
    match crate::routing::lookup::lookup(&state.storage, name.as_deref()) {
        Ok(mbs) => ServerMsg::LookupResult { mailboxes: mbs },
        Err(e) => ServerMsg::Error {
            code: "lookup_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_leave(state: &AppState, headers: &HeaderMap, _purge: bool) -> ServerMsg {
    let session = match super::authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    if let Err(e) = mailbox_db::mark_left(&state.storage, &session.address) {
        return ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        };
    }
    if let Err(e) = session_file_mod::remove(&state.dot_agtalk, &session.name) {
        return ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        };
    }
    if let Some(pid) = session.pid {
        let _ = agents_map::remove_pid(&state.dot_agtalk, pid);
    }
    crate::mem::index::remove(&state.storage, &session.address);

    ServerMsg::Pong
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
