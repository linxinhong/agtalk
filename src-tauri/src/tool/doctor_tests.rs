use super::*;
use crate::config::AgConfig;
use crate::identity::session_file::SessionFile;
use crate::identity::{mailbox, relations, session_file};
use crate::storage::Storage;
use std::ffi::OsString;
use tempfile::TempDir;

struct EnvGuard(Option<OsString>);

impl EnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os(crate::paths::CONFIG_DIR_ENV);
        std::env::set_var(crate::paths::CONFIG_DIR_ENV, path);
        Self(previous)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.0 {
            std::env::set_var(crate::paths::CONFIG_DIR_ENV, p);
        } else {
            std::env::remove_var(crate::paths::CONFIG_DIR_ENV);
        }
    }
}

fn test_ctx(tmp: &TempDir) -> (DoctorContext, EnvGuard) {
    let guard = EnvGuard::set(tmp.path());
    let dot = tmp.path().join(".agtalk");
    std::fs::create_dir_all(&dot).unwrap();
    let mut config = AgConfig::default();
    // 使用端口 0 避免与真实 daemon 或其他测试冲突；doctor 检查只尝试 bind，不建立长期服务。
    config.http_port = 0;
    let storage = Storage::open_in_memory().unwrap();
    let ctx = DoctorContext::new(dot, config, Some(storage), None);
    (ctx, guard)
}

fn find_check<'a>(checks: &'a [DiagnosisCheck], name: &str) -> Option<&'a DiagnosisCheck> {
    checks.iter().find(|c| c.name == name)
}

#[test]
fn doctor_no_identity_skips_identity_checks() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let msg = run(ctx);
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert!(find_check(&checks, "runtime.binary").is_some());
    assert_eq!(
        find_check(&checks, "identity.session_file").unwrap().status,
        "skip"
    );
    assert_eq!(
        find_check(&checks, "message.read_ready").unwrap().status,
        "skip"
    );
}

#[test]
fn doctor_with_session_resolves_identity() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let session = SessionFile {
        version: 2,
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();

    let msg = run(ctx.clone());
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert_eq!(
        find_check(&checks, "identity.session_file").unwrap().status,
        "ok"
    );
    assert_eq!(
        find_check(&checks, "identity.address").unwrap().status,
        "ok"
    );
    assert_eq!(
        find_check(&checks, "identity.db_mailbox").unwrap().status,
        "error" // mailbox 不在 DB
    );
    assert_eq!(
        find_check(&checks, "message.read_ready").unwrap().status,
        "ok" // 身份可解析即认为 read 就绪
    );
}

fn write_nora_session(ctx: &DoctorContext, address: &str) {
    let session = SessionFile {
        version: 2,
        address: address.to_string(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();
}

#[test]
fn doctor_warns_on_relations_owner_mismatch() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);
    write_nora_session(&ctx, "550e8400-e29b-41d4-a716-446655440000");

    // relations.json 的 owner.address 与当前身份不一致（模拟曾写入错误 workspace）。
    let rf = relations::RelationsFile {
        version: 2,
        owner: relations::RelationOwner {
            name: "nora".to_string(),
            address: "00000000-0000-0000-0000-000000000000".to_string(),
        },
        peers: Default::default(),
    };
    relations::write(&ctx.dot_agtalk, "nora", &rf).unwrap();

    let msg = run(ctx.clone());
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert_eq!(
        find_check(&checks, "identity.agent_dir_writable")
            .unwrap()
            .status,
        "ok"
    );
    let rel = find_check(&checks, "identity.relations_file").unwrap();
    assert_eq!(rel.status, "warn");
    assert!(rel.message.contains("owner.address"));
}

#[test]
fn doctor_relations_file_ok_when_owner_matches() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);
    let addr = "550e8400-e29b-41d4-a716-446655440000";
    write_nora_session(&ctx, addr);

    let rf = relations::RelationsFile {
        version: 2,
        owner: relations::RelationOwner {
            name: "nora".to_string(),
            address: addr.to_string(),
        },
        peers: Default::default(),
    };
    relations::write(&ctx.dot_agtalk, "nora", &rf).unwrap();

    let msg = run(ctx.clone());
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    let rel = find_check(&checks, "identity.relations_file").unwrap();
    assert_eq!(rel.status, "ok");
}

#[test]
fn doctor_aggregates_error_when_daemon_down() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let msg = run(ctx);
    let (status, checks) = match msg {
        ServerMsg::ToolDiagnosis { status, checks, .. } => (status, checks),
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert_eq!(status, "error");
    assert_eq!(
        find_check(&checks, "daemon.status_file").unwrap().status,
        "error"
    );
}

#[test]
fn doctor_daemon_stopped_summary() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let msg = run(ctx);
    let (status, summary, root_causes, actions) = match msg {
        ServerMsg::ToolDiagnosis {
            status,
            summary,
            root_causes,
            actions,
            ..
        } => (status, summary, root_causes, actions),
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert_eq!(status, "error");
    assert_eq!(summary, "local agent bus unavailable");
    assert_eq!(root_causes.len(), 1);
    assert_eq!(root_causes[0].id, "daemon.stopped");
    assert_eq!(actions, vec!["agtalk daemon start"]);
}

#[test]
fn doctor_daemon_stopped_suppresses_derived_errors() {
    let tmp = TempDir::new().unwrap();
    let (_ctx, _guard) = test_ctx(&tmp);
    // 无 storage 时 message.db 会 error，但 daemon stopped 场景下不应进入 root causes。
    let dot = tmp.path().join(".agtalk");
    std::fs::create_dir_all(&dot).unwrap();
    let mut config = AgConfig::default();
    config.http_port = 0;
    let ctx = DoctorContext::new(dot, config, None, None);

    let msg = run(ctx);
    let root_causes = match msg {
        ServerMsg::ToolDiagnosis { root_causes, .. } => root_causes,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    let ids: Vec<_> = root_causes.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["daemon.stopped"]);
}

#[test]
fn doctor_stale_mailbox_merged_when_daemon_stopped() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let session = SessionFile {
        version: 2,
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        name: "tester".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "tester", &session).unwrap();

    let msg = run(ctx);
    let (status, summary, root_causes, actions) = match msg {
        ServerMsg::ToolDiagnosis {
            status,
            summary,
            root_causes,
            actions,
            ..
        } => (status, summary, root_causes, actions),
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    // 测试环境没有 daemon，因此最高优先级根因是 daemon.stopped；
    // 但仍应合并出 identity.stale_mailbox。
    assert_eq!(status, "error");
    assert_eq!(summary, "local agent bus unavailable");
    let ids: Vec<_> = root_causes.iter().map(|c| c.id.as_str()).collect();
    assert!(
        ids.contains(&"daemon.stopped"),
        "expected daemon.stopped in {:?}",
        ids
    );
    assert!(
        ids.contains(&"identity.stale_mailbox"),
        "expected identity.stale_mailbox in {:?}",
        ids
    );
    assert!(
        !ids.contains(&"identity.db_mailbox"),
        "identity.db_mailbox should be merged"
    );
    assert!(
        !ids.contains(&"message.mailbox"),
        "message.mailbox should be merged"
    );
    assert_eq!(
        actions,
        vec!["agtalk daemon start", "agtalk id join tester"]
    );
}

#[test]
fn doctor_context_extracts_identity_and_pending() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let session = SessionFile {
        version: 2,
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();

    if let Some(storage) = &ctx.storage {
        mailbox::create(storage, "nora", "前端", "projA").unwrap();
    }

    let msg = run(ctx);
    let context = match msg {
        ServerMsg::ToolDiagnosis { context, .. } => context,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    assert_eq!(context.identity, Some("nora".to_string()));
    assert_eq!(
        context.address,
        Some("550e8400-e29b-41d4-a716-446655440000".to_string())
    );
    assert_eq!(context.pending, Some(0));
}

#[test]
fn doctor_summary_computes_ok() {
    let checks = vec![check(
        "runtime",
        "runtime.binary",
        "ok",
        "ok",
        None,
        None,
        serde_json::Value::Null,
    )];
    assert_eq!(
        compute_summary("ok", &checks),
        "agent environment is healthy"
    );
}

#[test]
fn doctor_summary_prefers_daemon_stopped() {
    let checks = vec![
        check(
            "daemon",
            "daemon.status_file",
            "error",
            "daemon.json 不存在",
            None,
            None,
            serde_json::Value::Null,
        ),
        check(
            "identity",
            "identity.db_mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
        check(
            "message",
            "message.mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
    ];
    assert_eq!(
        compute_summary("error", &checks),
        "local agent bus unavailable"
    );
}

#[test]
fn doctor_summary_detects_stale_mailbox() {
    let checks = vec![
        check(
            "identity",
            "identity.db_mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
        check(
            "message",
            "message.mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
    ];
    assert_eq!(
        compute_summary("error", &checks),
        "session exists but mailbox missing in DB"
    );
}

#[test]
fn doctor_root_causes_merge_stale_mailbox() {
    let checks = vec![
        check(
            "identity",
            "identity.db_mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
        check(
            "message",
            "message.mailbox",
            "error",
            "missing",
            None,
            None,
            serde_json::Value::Null,
        ),
    ];
    let identity = Some(ResolvedIdentity {
        name: "tester".to_string(),
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        session_path: std::path::PathBuf::new(),
        source: "test".to_string(),
    });
    let causes = compute_root_causes(&checks, &identity);
    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0].id, "identity.stale_mailbox");
    assert_eq!(causes[0].command, Some("agtalk id join tester".to_string()));
}

#[test]
fn doctor_actions_dedup_and_use_identity_name() {
    let identity = Some(ResolvedIdentity {
        name: "tester".to_string(),
        address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        session_path: std::path::PathBuf::new(),
        source: "test".to_string(),
    });
    let causes = vec![
        RootCause {
            id: "daemon.stopped".to_string(),
            status: "error".to_string(),
            message: "stopped".to_string(),
            command: Some("agtalk daemon start".to_string()),
        },
        RootCause {
            id: "identity.stale_mailbox".to_string(),
            status: "error".to_string(),
            message: "stale".to_string(),
            command: Some("agtalk id join tester".to_string()),
        },
    ];
    let actions = compute_actions(&causes, &identity);
    assert_eq!(
        actions,
        vec!["agtalk daemon start", "agtalk id join tester"]
    );
}

#[test]
fn doctor_plugin_missing_sets_config_command() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let address = "550e8400-e29b-41d4-a716-446655440000".to_string();
    let session = SessionFile {
        version: 2,
        address: address.clone(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "plugin:missing".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();
    if let Some(storage) = ctx.storage.as_ref() {
        mailbox::revive_with_notify(
            storage,
            &address,
            "nora",
            "前端",
            "",
            "plugin:missing",
            &serde_json::json!({"type":"plugin","name":"missing","endpoint":null}),
            "",
        )
        .unwrap();
    }

    let msg = run(ctx);
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    let plugin_check = find_check(&checks, "notify.plugin.binary").unwrap();
    assert_eq!(plugin_check.status, "error");
    assert!(plugin_check
        .command
        .as_ref()
        .unwrap()
        .contains("agtalk config set notify.plugins.missing.path"));
}

#[test]
fn doctor_plugin_not_executable_sets_config_command() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("not-executable.sh");
    std::fs::write(&plugin_path, "#!/bin/sh\n").unwrap();

    let mut config = AgConfig::default();
    config.http_port = 0;
    config.notify.plugins.insert(
        "bad".to_string(),
        crate::config::NotifyPluginEntry {
            path: "not-executable.sh".to_string(),
            timeout_ms: None,
        },
    );
    config.save().unwrap();

    let address = "550e8400-e29b-41d4-a716-446655440000".to_string();
    let session = SessionFile {
        version: 2,
        address: address.clone(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "plugin:bad".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();
    if let Some(storage) = ctx.storage.as_ref() {
        mailbox::revive_with_notify(
            storage,
            &address,
            "nora",
            "前端",
            "",
            "plugin:bad",
            &serde_json::json!({"type":"plugin","name":"bad","endpoint":null}),
            "",
        )
        .unwrap();
    }

    let msg = run(ctx);
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    let plugin_check = find_check(&checks, "notify.plugin.binary").unwrap();
    assert_eq!(plugin_check.status, "error");
    assert!(plugin_check
        .command
        .as_ref()
        .unwrap()
        .contains("agtalk config set notify.plugins.bad.path"));
}

#[test]
fn doctor_warns_when_notify_none_but_environment_ready() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _guard) = test_ctx(&tmp);

    // 构造一个可用的 mock plugin 并加入 PATH
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-zellij");
    let script = "#!/bin/sh\nif [ \"$1\" = \"discover\" ]; then echo '{\"version\":1,\"type\":\"notify_endpoint\",\"channel\":\"zellij\",\"ready\":true,\"endpoint\":{\"pane\":\"1\"},\"message\":\"ok\"}'; fi\n";
    std::fs::write(&plugin_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prev_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = std::env::split_paths(&prev_path).collect::<Vec<_>>();
    paths.push(plugins_dir);
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

    let address = "550e8400-e29b-41d4-a716-446655440000".to_string();
    let session = SessionFile {
        version: 2,
        address: address.clone(),
        name: "nora".to_string(),
        intro: "前端".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
        registered_by: Some("agtalk".to_string()),
        notify: session_file::SessionNotify {
            channel: "none".to_string(),
            endpoint: serde_json::Value::Null,
        },
    };
    session_file::write(&ctx.dot_agtalk, "nora", &session).unwrap();
    if let Some(storage) = ctx.storage.as_ref() {
        mailbox::revive_with_notify(
            storage,
            &address,
            "nora",
            "前端",
            "",
            "none",
            &serde_json::json!({"type":"none"}),
            "",
        )
        .unwrap();
    }

    let msg = run(ctx);
    let checks = match msg {
        ServerMsg::ToolDiagnosis { checks, .. } => checks,
        other => panic!("expected ToolDiagnosis, got {:?}", other),
    };

    let channel_check = find_check(&checks, "notify.channel").unwrap();
    assert_eq!(channel_check.status, "warn");
    assert!(channel_check
        .command
        .as_ref()
        .unwrap()
        .contains("agtalk id join nora --notify auto"));

    std::env::set_var("PATH", prev_path);
}
