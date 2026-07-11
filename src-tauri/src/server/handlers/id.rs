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

    /// 隔离真实环境的 RAII guard：临时 AGTALK_CONFIG_DIR + 临时 PATH 前缀。
    ///
    /// 创建 `<tmp>/plugins/agtalk-notify-<name>` mock 二进制，并把临时目录设为
    /// `AGTALK_CONFIG_DIR`，同时在 PATH 最前面加入 `<tmp>/plugins`。Drop 时恢复。
    struct MockPluginGuard {
        _tmp: TempDir,
        prev_config_dir: Option<std::ffi::OsString>,
        prev_path: Option<std::ffi::OsString>,
    }

    impl Drop for MockPluginGuard {
        fn drop(&mut self) {
            if let Some(dir) = &self.prev_config_dir {
                std::env::set_var(crate::paths::CONFIG_DIR_ENV, dir);
            } else {
                std::env::remove_var(crate::paths::CONFIG_DIR_ENV);
            }
            if let Some(path) = &self.prev_path {
                std::env::set_var("PATH", path);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }

    /// 创建临时 mock plugin 目录并隔离全局配置目录。
    /// 返回 guard，调用方用 `_guard` 持有；即使 panic 也会在作用域结束时恢复环境。
    fn mock_plugin_env(name: &str) -> MockPluginGuard {
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

        let prev_config_dir = std::env::var_os(crate::paths::CONFIG_DIR_ENV);
        std::env::set_var(crate::paths::CONFIG_DIR_ENV, tmp.path());

        let prev_path = std::env::var_os("PATH");
        let mut paths =
            std::env::split_paths(&prev_path.clone().unwrap_or_default()).collect::<Vec<_>>();
        paths.insert(0, plugins_dir);
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

        MockPluginGuard {
            _tmp: tmp,
            prev_config_dir,
            prev_path,
        }
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
    fn join_persists_workspace_root_on_create_and_revive() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        // agent 的 .agtalk 根与 daemon 的 state.dot_agtalk 不同（模拟跨 workspace）。
        let agent_tmp = TempDir::new().unwrap();
        let agent_dot = agent_tmp.path().join(".agtalk");

        let msg = handle_join(
            &state,
            &agent_dot,
            Some("agentx".into()),
            Some("intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );
        let address = match msg {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };

        let mb = mailbox_db::get_by_address(&state.storage, &address)
            .unwrap()
            .unwrap();
        assert_eq!(
            mb.workspace_root,
            agent_dot.to_string_lossy(),
            "create 应持久化 agent 的绝对 .agtalk 根"
        );
        assert_ne!(
            mb.workspace_root,
            state.dot_agtalk.to_string_lossy(),
            "workspace_root 不应等于 daemon 的 state.dot_agtalk"
        );

        // 复用身份（revive 路径）也应刷新 workspace_root。
        let msg2 = handle_join(
            &state,
            &agent_dot,
            Some("agentx".into()),
            Some("intro2".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );
        let address2 = match msg2 {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };
        assert_eq!(address, address2, "同 name 复用应保持同一 address");
        let mb2 = mailbox_db::get_by_address(&state.storage, &address2)
            .unwrap()
            .unwrap();
        assert_eq!(
            mb2.workspace_root,
            agent_dot.to_string_lossy(),
            "revive 应刷新 workspace_root"
        );
    }

    #[test]
    fn join_creates_identity_with_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let msg = handle_join(
            &state,
            &state.dot_agtalk,
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
                notify_channel,
                ..
            } => {
                assert!(!address.is_empty());
                assert_eq!(name, "reviewer");
                assert_eq!(intro, "设计评审专家");
                assert_eq!(notify_channel, "none");
            }
            other => panic!("expected Identity, got {:?}", other),
        }

        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.intro, "设计评审专家");
        // v2 session 不再包含 workspace 字段
    }

    #[test]
    fn join_reuses_session_address_and_updates_intro() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let first = handle_join(
            &state,
            &state.dot_agtalk,
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
            &state.dot_agtalk,
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
                ..
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
            &state.dot_agtalk,
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
    fn join_without_notify_upgrades_old_none_to_plugin_when_ready() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        // 首次以 none 创建
        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("初代 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );
        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.notify_channel(), "none");

        // 再次 join 不传 notify，应升级为 plugin:zellij
        let msg = handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            None,
            "auto".into(),
            None,
            pid,
            start_time,
        );
        match msg {
            ServerMsg::Identity {
                notify_channel,
                notify_ready,
                ..
            } => {
                assert_eq!(notify_channel, "plugin:zellij");
                assert!(notify_ready);
            }
            other => panic!("expected Identity, got {:?}", other),
        }
        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.notify_channel(), "plugin:zellij");
    }

    #[test]
    fn join_without_notify_refreshes_existing_plugin_endpoint() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("初代 intro".into()),
            "plugin:zellij".into(),
            None,
            pid,
            start_time,
        );
        let first_session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(first_session.notify_channel(), "plugin:zellij");

        // 不传 notify 再次 join，channel 保持 plugin:zellij，endpoint 会被刷新
        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            None,
            "auto".into(),
            None,
            pid,
            start_time,
        );
        let second_session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(second_session.notify_channel(), "plugin:zellij");
    }

    #[test]
    fn join_explicit_notify_none_persists_none() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("初代 intro".into()),
            "plugin:zellij".into(),
            None,
            pid,
            start_time,
        );

        let msg = handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            None,
            "none".into(),
            None,
            pid,
            start_time,
        );
        match msg {
            ServerMsg::Identity {
                notify_channel,
                notify_ready,
                ..
            } => {
                assert_eq!(notify_channel, "none");
                assert!(!notify_ready);
            }
            other => panic!("expected Identity, got {:?}", other),
        }
        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        assert_eq!(session.notify_channel(), "none");
    }

    #[test]
    fn join_explicit_plugin_not_ready_returns_error() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let msg = handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("intro".into()),
            "plugin:missing-plugin".into(),
            None,
            pid,
            start_time,
        );
        match msg {
            ServerMsg::Error { code, .. } => {
                assert_eq!(code, "join_failed");
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn lookup_includes_notify_zellij() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
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
    }

    #[test]
    fn lookup_includes_notify_plugin() {
        let _guard = mock_plugin_env("macos");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
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
    }

    #[test]
    fn lookup_stale_session_uses_db_notify() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "plugin:zellij".into(),
            None,
            pid,
            start_time,
        );

        // 篡改 session address，使其与 mailbox address 不匹配；DB 仍是 canonical 源。
        let mut session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        session.address = "00000000-0000-0000-0000-000000000000".to_string();
        session_file::write(&state.dot_agtalk, "reviewer", &session).unwrap();

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.notify, "plugin:zellij");
                assert!(mb.notify_ready);
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
            &state.dot_agtalk,
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

        // 默认 purge=false，不删除本地 session 目录
        let msg = handle_leave(&state, &headers, None, false);
        match msg {
            ServerMsg::IdentityLeft {
                name,
                removed_session,
                purge,
                ..
            } => {
                assert_eq!(name, "reviewer");
                assert!(!removed_session);
                assert!(!purge);
            }
            other => panic!("expected IdentityLeft, got {:?}", other),
        }

        // session 目录应保留
        assert!(session_file::read(&state.dot_agtalk, "reviewer").is_ok());
        // pid anchor 应被清理
        assert!(agents_map::get_by_pid(&state.dot_agtalk, 12345)
            .unwrap()
            .is_none());
        assert!(agents_map::get_by_pid(&state.dot_agtalk, 12346)
            .unwrap()
            .is_none());
    }

    #[test]
    fn leave_by_address_targets_specific_mailbox() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let alice_addr = match handle_join(
            &state,
            &state.dot_agtalk,
            Some("alice".into()),
            Some("alice intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        ) {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };

        let bob_addr = match handle_join(
            &state,
            &state.dot_agtalk,
            Some("bob".into()),
            Some("bob intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        ) {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };

        let msg = handle_leave(&state, &HeaderMap::new(), Some(bob_addr.clone()), false);
        match msg {
            ServerMsg::IdentityLeft { address, name, .. } => {
                assert_eq!(address, bob_addr);
                assert_eq!(name, "bob");
            }
            other => panic!("expected IdentityLeft, got {:?}", other),
        }

        assert!(mailbox_db::get_by_address(&state.storage, &bob_addr)
            .unwrap()
            .is_none());
        assert!(mailbox_db::get_by_address(&state.storage, &alice_addr)
            .unwrap()
            .is_some());
        assert!(session_file::read(&state.dot_agtalk, "bob").is_ok());
        assert!(session_file::read(&state.dot_agtalk, "alice").is_ok());
    }

    #[test]
    fn leave_by_address_with_purge_removes_session() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        let addr = match handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        ) {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        };

        let msg = handle_leave(&state, &HeaderMap::new(), Some(addr), true);
        match msg {
            ServerMsg::IdentityLeft {
                removed_session,
                purge,
                ..
            } => {
                assert!(removed_session);
                assert!(purge);
            }
            other => panic!("expected IdentityLeft, got {:?}", other),
        }

        assert!(session_file::read(&state.dot_agtalk, "reviewer").is_err());
    }

    #[test]
    fn lookup_missing_session_uses_db_notify() {
        let _guard = mock_plugin_env("zellij");
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "plugin:zellij".into(),
            None,
            pid,
            start_time,
        );

        // 删除 session 文件，模拟 session 缺失；DB 仍是 canonical 源。
        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();

        let msg = handle_lookup(&state, Some("reviewer".into()));
        match msg {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                let mb = &mailboxes[0];
                assert_eq!(mb.notify, "plugin:zellij");
                assert!(mb.notify_ready);
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
            &state.dot_agtalk,
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

        let msg = handle_cleanup(&state, &state.dot_agtalk, false);
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
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        let session_path = state.dot_agtalk.join("reviewer").join("session.json");
        std::fs::remove_file(session_path).unwrap();

        let msg = handle_cleanup(&state, &state.dot_agtalk, true);
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
            version: 2,
            address: "00000000-0000-0000-0000-000000000001".to_string(),
            name: "orphan".to_string(),
            intro: "no mailbox".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            registered_by: None,
            notify: SessionNotify {
                channel: "none".to_string(),
                endpoint: serde_json::Value::Null,
            },
        };
        session_file::write(&state.dot_agtalk, "orphan", &session).unwrap();

        let msg = handle_cleanup(&state, &state.dot_agtalk, true);
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
    fn cleanup_removes_stale_session_when_mailbox_left() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
            Some("reviewer".into()),
            Some("展示用 intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        );

        // 把 mailbox 标记为 left，但保留 session，制造 stale_session。
        let session = session_file::read(&state.dot_agtalk, "reviewer").unwrap();
        crate::identity::mailbox::mark_left(&state.storage, &session.address).unwrap();

        let msg = handle_cleanup(&state, &state.dot_agtalk, true);
        match msg {
            ServerMsg::CleanupResult {
                dry_run,
                removed,
                skipped,
            } => {
                assert!(!dry_run);
                let reasons: Vec<_> = removed.iter().map(|i| i.reason.as_str()).collect();
                assert!(
                    reasons.contains(&"stale_session"),
                    "expected stale_session in {:?}",
                    reasons
                );
                assert!(skipped.is_empty());
            }
            other => panic!("expected CleanupResult, got {:?}", other),
        }

        assert!(session_file::read(&state.dot_agtalk, "reviewer").is_err());
    }

    #[test]
    fn cleanup_removes_stale_pid_anchors() {
        let (state, _tmp) = test_state();
        let (pid, start_time) = current_pid_start_time();

        handle_join(
            &state,
            &state.dot_agtalk,
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

        let msg = handle_cleanup(&state, &state.dot_agtalk, true);
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
