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
    headers.insert(
        "X-AgTalk-Workspace-Root",
        state.dot_agtalk.to_string_lossy().as_ref().parse().unwrap(),
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
    headers.insert(
        "X-AgTalk-Workspace-Root",
        state.dot_agtalk.to_string_lossy().as_ref().parse().unwrap(),
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
