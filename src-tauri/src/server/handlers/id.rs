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

            let base_channel = if notify.eq_ignore_ascii_case("auto") {
                if session.notify_channel.is_empty()
                    || session.notify_channel.eq_ignore_ascii_case("auto")
                {
                    None
                } else {
                    Some(session.notify_channel.clone())
                }
            } else {
                Some(notify.clone())
            };
            let (notify_channel, notify_target) =
                resolve_notify(&base_channel.unwrap_or_else(|| "auto".to_string()));

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

            let (notify_channel, notify_target) = resolve_notify(&notify);

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

    ServerMsg::Identity {
        address,
        name,
        workspace: session.workspace,
        intro: session.intro,
    }
}

pub fn handle_show(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let session = match super::authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let intro = session_file_mod::read(&state.dot_agtalk, &session.name)
        .map(|s| s.intro)
        .unwrap_or_default();
    ServerMsg::Identity {
        address: session.address,
        name: session.name,
        workspace: session.workspace,
        intro,
    }
}

pub fn handle_lookup(state: &AppState, name: Option<String>) -> ServerMsg {
    match crate::routing::lookup::lookup(&state.storage, name.as_deref()) {
        Ok(mbs) => {
            let mailboxes = mbs
                .iter()
                .map(|mb| build_lookup_mailbox(&state.dot_agtalk, mb))
                .collect();
            ServerMsg::LookupResult { mailboxes }
        }
        Err(e) => ServerMsg::Error {
            code: "lookup_failed".into(),
            message: e.to_string(),
        },
    }
}

fn build_lookup_mailbox(
    dot_agtalk: &std::path::Path,
    mb: &crate::identity::mailbox::Mailbox,
) -> crate::proto::LookupMailbox {
    let (channel, notify, ready) = match crate::identity::session_file::read(dot_agtalk, &mb.name) {
        Ok(session) if session.address == mb.address => {
            let ready = !session.notify_channel.eq_ignore_ascii_case("none")
                && !session.notify_channel.is_empty();
            let notify = notify_summary(&session.notify_channel);
            (session.notify_channel, notify, ready)
        }
        _ => ("unknown".to_string(), "unknown".to_string(), false),
    };
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

/// 统一解析 notify 参数：auto/none/plugin:<name>。
/// 对 plugin 通道会调用 discover 获取 endpoint；若 discover 失败或插件不存在，
/// 不阻塞 join，降级为 none。
fn resolve_notify(notify: &str) -> (String, NotifyTarget) {
    if notify.eq_ignore_ascii_case("none") {
        return ("none".to_string(), NotifyTarget::None);
    }
    if notify.eq_ignore_ascii_case("auto") {
        return notify::auto_detect();
    }
    if let Some(plugin_name) = notify.strip_prefix("plugin:") {
        if let Some(channel) = notify::channel_from_name(notify) {
            match channel.discover() {
                Ok(Some(endpoint)) if endpoint.ready => {
                    return (
                        notify.to_string(),
                        NotifyTarget::Plugin {
                            name: plugin_name.to_string(),
                            endpoint: endpoint.endpoint,
                        },
                    );
                }
                Ok(Some(endpoint)) => {
                    tracing::warn!(
                        "notify plugin {} discover 返回 not ready: {}",
                        plugin_name,
                        endpoint.message
                    );
                }
                Ok(None) => {
                    tracing::warn!("notify plugin {} discover 返回空", plugin_name);
                }
                Err(e) => {
                    tracing::warn!("notify plugin {} discover 失败: {}", plugin_name, e);
                }
            }
        }
        // discover 失败或不 ready 时降级为 none，不阻塞 join。
        return ("none".to_string(), NotifyTarget::None);
    }
    // 未知通道统一降级为 none。
    tracing::warn!("未知 notify 通道 '{}', 降级为 none", notify);
    ("none".to_string(), NotifyTarget::None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::session_file;
    use crate::storage::Storage;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    use sysinfo::{Pid, System};
    use tempfile::TempDir;

    fn test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let state = AppState::new(storage, AgConfig::default(), dot);
        (state, tmp)
    }

    /// 创建临时 mock plugin 目录，把 `<tmp>/plugins` 加入 PATH，并写入 `agtalk-notify-<name>`。
    /// 返回 (TempDir, prev_path)；调用方需用 `_tmp` 持有 TempDir，并在测试结束后恢复 PATH。
    fn mock_plugin_env(name: &str) -> (TempDir, std::ffi::OsString) {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let plugin_path = plugins_dir.join(format!("agtalk-notify-{}", name));
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"discover\" ]; then echo '{{\"version\":1,\"type\":\"notify_endpoint\",\"channel\":\"{}\",\"ready\":true,\"endpoint\":{{\"pane\":\"1\"}},\"message\":\"ok\"}}'; fi\n",
            name
        );
        std::fs::write(&plugin_path, script).unwrap();
        #[cfg(unix)]
        {
            std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let prev_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = std::env::split_paths(&prev_path).collect::<Vec<_>>();
        paths.push(plugins_dir);
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        (tmp, prev_path)
    }

    fn current_pid_start_time() -> (u32, u64) {
        let pid = std::process::id();
        let mut sys = System::new_all();
        sys.refresh_processes();
        let start = sys
            .process(Pid::from(pid as usize))
            .map(|p| p.start_time())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            });
        (pid, start)
    }

    #[test]
    fn join_creates_identity_with_intro_and_workspace() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let msg = handle_join(
            &state,
            Some("reviewer".into()),
            Some("设计评审专家".into()),
            Some("agtalk".into()),
            "none".into(),
            pid,
            start_time,
        );

        match msg {
            ServerMsg::Identity {
                address,
                name,
                workspace,
                intro,
            } => {
                assert!(!address.is_empty());
                assert_eq!(name, "reviewer");
                assert_eq!(workspace, "agtalk");
                assert_eq!(intro, "设计评审专家");
            }
            other => panic!("expected Identity, got {:?}", other),
        }

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.intro, "设计评审专家");
        assert_eq!(session.workspace, "agtalk");
    }

    #[test]
    fn join_reuses_session_address_and_updates_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let first = handle_join(
            &state,
            Some("reviewer".into()),
            Some("初代 intro".into()),
            Some("agtalk".into()),
            "none".into(),
            pid,
            start_time,
        );
        let first_address = match first {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };

        let second = handle_join(
            &state,
            Some("reviewer".into()),
            Some("更新后的 intro".into()),
            None,
            "none".into(),
            pid,
            start_time,
        );

        match second {
            ServerMsg::Identity {
                address,
                name,
                workspace,
                intro,
            } => {
                assert_eq!(address, first_address);
                assert_eq!(name, "reviewer");
                assert_eq!(workspace, "agtalk");
                assert_eq!(intro, "更新后的 intro");
            }
            other => panic!("expected Identity, got {:?}", other),
        }

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.intro, "更新后的 intro");
    }

    #[test]
    fn show_returns_session_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            Some("agtalk".into()),
            "none".into(),
            pid,
            start_time,
        );

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
        headers.insert("X-AgTalk-Pid", pid.to_string().parse().unwrap());
        headers.insert(
            "X-AgTalk-Start-Time",
            start_time.to_string().parse().unwrap(),
        );

        let msg = handle_show(&state, &headers);
        match msg {
            ServerMsg::Identity { intro, .. } => {
                assert_eq!(intro, "展示用 intro");
            }
            other => panic!("expected Identity, got {:?}", other),
        }
    }

    #[test]
    fn lookup_includes_notify_zellij() {
        let (_plugin_tmp, prev_path) = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            Some("agtalk".into()),
            "plugin:zellij".into(),
            pid,
            start_time,
        );

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.name, "reviewer");
                assert_eq!(mb.notify_channel, "plugin:zellij");
                assert_eq!(mb.notify, "plugin:zellij");
                assert!(mb.notify_ready);
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }

        std::env::set_var("PATH", prev_path);
    }

    #[test]
    fn lookup_includes_notify_plugin() {
        let (_plugin_tmp, prev_path) = mock_plugin_env("macos");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            Some("agtalk".into()),
            "plugin:macos".into(),
            pid,
            start_time,
        );

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.notify_channel, "plugin:macos");
                assert_eq!(mb.notify, "plugin:macos");
                assert!(mb.notify_ready);
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }

        std::env::set_var("PATH", prev_path);
    }

    #[test]
    fn lookup_stale_session_returns_unknown_notify() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            Some("agtalk".into()),
            "zellij".into(),
            pid,
            start_time,
        );

        // 篡改 session address，使其与 mailbox address 不匹配
        let mut session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        session.address = "00000000-0000-0000-0000-000000000000".to_string();
        session_file::write(&state.dot_agtalk, "reviewer", &session).unwrap();

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.notify, "unknown");
                assert!(!mb.notify_ready);
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[test]
    fn lookup_missing_session_returns_unknown_notify() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            Some("agtalk".into()),
            "zellij".into(),
            pid,
            start_time,
        );

        // 删除 session 文件，模拟 session 缺失
        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.notify, "unknown");
                assert!(!mb.notify_ready);
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }
}
