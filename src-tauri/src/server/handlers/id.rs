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

#[allow(clippy::too_many_arguments)]
pub fn handle_join(
    state: &AppState,
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
    let existing_session = session_file_mod::read(&state.dot_agtalk, &name).ok();

    let (address, final_intro, notify_channel, notify_target) =
        if let Some(ref session) = existing_session {
            let final_intro = intro.unwrap_or_else(|| session.intro.clone());

            if let Err(e) =
                mailbox_db::revive(&state.storage, &session.address, &name, &final_intro, "")
            {
                return ServerMsg::Error {
                    code: "join_failed".into(),
                    message: e.to_string(),
                };
            }

            let base_channel = if notify.eq_ignore_ascii_case("auto") {
                if session.notify_channel.is_empty()
                    || session.notify_channel.eq_ignore_ascii_case("auto")
                {
                    "auto".to_string()
                } else {
                    session.notify_channel.clone()
                }
            } else {
                notify.clone()
            };
            let (notify_channel, notify_target) = resolve_notify(
                &base_channel,
                notify_endpoint.clone(),
                Some(&session.notify_target),
            );

            (
                session.address.clone(),
                final_intro,
                notify_channel,
                notify_target,
            )
        } else {
            let final_intro = intro.unwrap_or_default();
            let address = match mailbox_db::create(&state.storage, &name, &final_intro, "") {
                Ok(addr) => addr,
                Err(e) => {
                    return ServerMsg::Error {
                        code: "join_failed".into(),
                        message: e.to_string(),
                    }
                }
            };

            let (notify_channel, notify_target) =
                resolve_notify(&notify, notify_endpoint.clone(), None);

            (address, final_intro, notify_channel, notify_target)
        };

    let command = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "agtalk".to_string());

    let session = SessionFile {
        address: address.clone(),
        name: name.clone(),
        workspace: "".to_string(),
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
    crate::mem::index::register(&state.storage, &address, &name, "", &memory_path);

    ServerMsg::Identity {
        address,
        name,
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
    let removed_session = session_file_mod::remove(&state.dot_agtalk, &session.name).is_ok();
    // 清理所有指向该 name 的 pid 锚点，避免 stale agents.json。
    let _ = agents_map::remove_by_name(&state.dot_agtalk, &session.name);
    crate::mem::index::remove(&state.storage, &session.address);

    ServerMsg::IdentityLeft {
        address: session.address,
        name: session.name,
        removed_session,
    }
}

/// 批量清理无效/未激活身份。
/// 默认 dry-run，返回候选列表；`execute=true` 时执行删除。
pub fn handle_cleanup(state: &AppState, execute: bool) -> ServerMsg {
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
    let session_names = match session_file_mod::list_session_names(&state.dot_agtalk) {
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
        if let Ok(session) = session_file_mod::read(&state.dot_agtalk, name) {
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
                let _ = session_file_mod::remove(&state.dot_agtalk, name);
                removed.push(item);
            } else {
                removed.push(item);
            }
        }
    }

    // 5. stale pid anchor：agents.json 指向的 session 已不存在。
    let valid_session_names: Vec<String> = session_by_name.keys().cloned().collect();
    match agents_map::cleanup_stale_pids(&state.dot_agtalk, &valid_session_names) {
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
/// discover 失败时，若已有同名的旧 endpoint 则保留，否则降级为 none，不阻塞 join。
fn resolve_notify(
    notify: &str,
    notify_endpoint: Option<serde_json::Value>,
    existing_target: Option<&NotifyTarget>,
) -> (String, NotifyTarget) {
    if notify.eq_ignore_ascii_case("none") {
        return ("none".to_string(), NotifyTarget::None);
    }
    if let Some(plugin_name) = notify.strip_prefix("plugin:") {
        // 优先 CLI 传来的 endpoint。
        if let Some(endpoint) = notify_endpoint {
            return (
                notify.to_string(),
                NotifyTarget::Plugin {
                    name: plugin_name.to_string(),
                    endpoint,
                },
            );
        }
        // 否则尝试 daemon 侧 discover（向后兼容无 CLI discover 的调用方）。
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
        // 若已有同名旧 endpoint，保留它（例如 session 复用时插件当前不可用）。
        if let Some(NotifyTarget::Plugin { name, endpoint }) = existing_target {
            if name == plugin_name {
                return (
                    notify.to_string(),
                    NotifyTarget::Plugin {
                        name: name.clone(),
                        endpoint: endpoint.clone(),
                    },
                );
            }
        }
        // 降级为 none。
        return ("none".to_string(), NotifyTarget::None);
    }
    if notify.eq_ignore_ascii_case("auto") {
        return notify::auto_detect();
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
    fn join_creates_identity_with_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let msg = handle_join(
            &state,
            Some("reviewer".into()),
            Some("设计评审专家".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        match msg {
            ServerMsg::Identity {
                address,
                name,
                intro,
            } => {
                assert!(!address.is_empty());
                assert_eq!(name, "reviewer");
                assert_eq!(intro, "设计评审专家");
            }
            other => panic!("expected Identity, got {:?}", other),
        }

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.intro, "设计评审专家");
        assert_eq!(session.workspace, "");
    }

    #[test]
    fn join_reuses_session_address_and_updates_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let first = handle_join(
            &state,
            Some("reviewer".into()),
            Some("初代 intro".into()),
            "none".into(),
            None,
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
            "none".into(),
            None,
            pid,
            start_time,
        );

        match second {
            ServerMsg::Identity {
                address,
                name,
                intro,
            } => {
                assert_eq!(address, first_address);
                assert_eq!(name, "reviewer");
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
            "none".into(),
            None,
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
            "plugin:zellij".into(),
            None,
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
            "plugin:macos".into(),
            None,
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
            "zellij".into(),
            None,
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
    fn leave_returns_identity_left_and_removes_all_pid_entries() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        // 额外注册几个指向 reviewer 的 pid
        agents_map::register_pid(&state.dot_agtalk, 12345, "reviewer", 1_700_000_000).unwrap();
        agents_map::register_pid(&state.dot_agtalk, 12346, "reviewer", 1_700_000_001).unwrap();

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
        headers.insert("X-AgTalk-Pid", pid.to_string().parse().unwrap());
        headers.insert(
            "X-AgTalk-Start-Time",
            start_time.to_string().parse().unwrap(),
        );

        let msg = handle_leave(&state, &headers, false);
        match msg {
            ServerMsg::IdentityLeft {
                name,
                removed_session,
                ..
            } => {
                assert_eq!(name, "reviewer");
                assert!(removed_session);
            }
            other => panic!("expected IdentityLeft, got {:?}", other),
        }

        assert!(session_file::read(&state.dot_agtalk, "reviewer").is_err());
        assert!(agents_map::get_by_pid(&state.dot_agtalk, 12345)
            .unwrap()
            .is_none());
        assert!(agents_map::get_by_pid(&state.dot_agtalk, 12346)
            .unwrap()
            .is_none());
    }

    #[test]
    fn lookup_missing_session_returns_unknown_notify() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "zellij".into(),
            None,
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

    #[test]
    fn cleanup_dry_run_lists_stale_mailbox() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        // 删除 session 但保留 DB mailbox，制造 stale mailbox
        // 同时 agents.json 中的 pid anchor 也会变成 stale。
        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();

        let msg = handle_cleanup(&state, false);
        match msg {
            ServerMsg::CleanupResult {
                dry_run,
                removed,
                skipped,
            } => {
                assert!(dry_run);
                let reasons: Vec<_> = removed.iter().map(|i| i.reason.as_str()).collect();
                assert!(reasons.contains(&"stale_mailbox"));
                assert!(reasons.contains(&"stale_pid_anchor"));
                assert!(skipped.is_empty());
            }
            other => panic!("expected CleanupResult, got {:?}", other),
        }
    }

    #[test]
    fn cleanup_execute_removes_stale_mailbox() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();

        let msg = handle_cleanup(&state, true);
        match msg {
            ServerMsg::CleanupResult {
                dry_run,
                removed,
                skipped,
            } => {
                assert!(!dry_run);
                let reasons: Vec<_> = removed.iter().map(|i| i.reason.as_str()).collect();
                assert!(reasons.contains(&"stale_mailbox"));
                assert!(reasons.contains(&"stale_pid_anchor"));
                assert!(skipped.is_empty());
            }
            other => panic!("expected CleanupResult, got {:?}", other),
        }

        // 清理后 lookup 不应再返回 reviewer（mailbox 已被标记 left）
        let msg = handle_lookup(&state, None);
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert!(mailboxes.is_empty());
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[test]
    fn cleanup_removes_stale_session() {
        let (state, _tmp) = test_state();

        // 直接写入一个无对应 mailbox 的 session 文件
        let session = session_file::SessionFile {
            address: "00000000-0000-0000-0000-000000000001".to_string(),
            name: "orphan".to_string(),
            workspace: "".to_string(),
            intro: "no mailbox".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            command: "".to_string(),
            notify_channel: "none".to_string(),
            notify_target: Default::default(),
        };
        session_file::write(&state.dot_agtalk, "orphan", &session).unwrap();

        let msg = handle_cleanup(&state, true);
        match msg {
            ServerMsg::CleanupResult {
                dry_run,
                removed,
                skipped,
            } => {
                assert!(!dry_run);
                let reasons: Vec<_> = removed.iter().map(|i| i.reason.as_str()).collect();
                assert!(reasons.contains(&"stale_session"));
                assert!(skipped.is_empty());
            }
            other => panic!("expected CleanupResult, got {:?}", other),
        }

        assert!(session_file::read(&state.dot_agtalk, "orphan").is_err());
    }

    #[test]
    fn cleanup_removes_stale_pid_anchors() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        // 删除 session，但保留指向 reviewer 的 pid anchor
        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();
        agents_map::register_pid(&state.dot_agtalk, 99999, "reviewer", start_time).unwrap();

        let msg = handle_cleanup(&state, true);
        match msg {
            ServerMsg::CleanupResult {
                dry_run,
                removed,
                skipped,
            } => {
                assert!(!dry_run);
                let reasons: Vec<_> = removed.iter().map(|i| i.reason.as_str()).collect();
                assert!(reasons.contains(&"stale_mailbox"));
                assert!(reasons.contains(&"stale_pid_anchor"));
                assert!(skipped.is_empty());

                assert!(removed
                    .iter()
                    .any(|i| { i.reason == "stale_pid_anchor" && i.name.contains("99999") }));
            }
            other => panic!("expected CleanupResult, got {:?}", other),
        }
    }
}
