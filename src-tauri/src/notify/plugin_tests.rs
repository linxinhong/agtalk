use super::*;
use crate::config::{AgConfig, NotifyConfig, NotifyPluginEntry};
use crate::notify::NotifyHint;
use crate::paths::CONFIG_DIR_ENV;
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn hint() -> NotifyHint {
    NotifyHint {
        from_name: "nora".to_string(),
        binary_path: "/usr/local/bin/agtalk".to_string(),
        agent_name: "codex".to_string(),
        agent_address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        message_id: "msg-123".to_string(),
        send_enter: true,
    }
}

fn setup_config(_tmp: &TempDir, plugins: HashMap<String, NotifyPluginEntry>) {
    let config = AgConfig {
        notify: NotifyConfig {
            plugins,
            ..Default::default()
        },
        ..Default::default()
    };
    config.save().unwrap();
}

fn env_guard(tmp: &TempDir) -> EnvGuard {
    EnvGuard::set(tmp.path())
}

struct EnvGuard {
    prev_config_dir: Option<std::ffi::OsString>,
    prev_path: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let prev_config_dir = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, path);

        let plugins_dir = path.join("plugins");
        let _ = std::fs::create_dir_all(&plugins_dir);
        let prev_path = std::env::var_os("PATH");
        let mut paths =
            std::env::split_paths(&prev_path.clone().unwrap_or_default()).collect::<Vec<_>>();
        paths.push(plugins_dir);
        if let Ok(joined) = std::env::join_paths(paths) {
            std::env::set_var("PATH", joined);
        }

        Self {
            prev_config_dir,
            prev_path,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.prev_config_dir {
            std::env::set_var(CONFIG_DIR_ENV, p);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
        if let Some(ref p) = self.prev_path {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }
    }
}

#[test]
fn payload_does_not_include_body_or_secret() {
    let payload = build_send_payload(&serde_json::json!({ "pane": "1" }), &hint());
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("nora"));
    assert!(json.contains("agtalk --as codex msg read"));
    assert!(json.contains("[\"--as\",\"codex\",\"msg\",\"read\"]"));
    assert!(json.contains("msg-123"));
    assert!(json.contains("[agtalk:msg] | from nora | exec:"));
    assert!(json.contains("\"send_enter\":true"));
    assert!(!json.contains("secret"));
    assert!(!json.contains("message body"));
}

#[test]
fn resolve_config_path_uses_absolute_as_is() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let path = PluginChannel::resolve_config_path("/usr/bin/plugin").unwrap();
    assert_eq!(path, PathBuf::from("/usr/bin/plugin"));
}

#[test]
fn resolve_config_path_resolves_filename_to_plugins_dir() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let path = PluginChannel::resolve_config_path("my-plugin").unwrap();
    assert_eq!(path, tmp.path().join("plugins").join("my-plugin"));
}

#[test]
fn resolve_config_path_resolves_relative_to_plugins_dir() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let path = PluginChannel::resolve_config_path("subdir/my-plugin").unwrap();
    assert_eq!(path, tmp.path().join("plugins").join("subdir/my-plugin"));
}

#[test]
fn resolve_config_path_rejects_parent_dir() {
    let err = PluginChannel::resolve_config_path("../escape").unwrap_err();
    assert!(err.to_string().contains("'..'"));
}

#[test]
fn validate_rejects_missing_file() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let path = tmp.path().join("plugins").join("missing");
    let err = PluginChannel::validate_binary(&path).unwrap_err();
    assert!(err.to_string().contains("不存在"));
}

#[test]
fn validate_rejects_directory() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_dir = plugins_dir.join("not-a-file");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    let err = PluginChannel::validate_binary(&plugin_dir).unwrap_err();
    assert!(err.to_string().contains("不是可执行文件"));
}

#[test]
fn validate_rejects_non_executable_file() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("not-executable.sh");
    std::fs::write(&plugin_path, "#!/bin/sh\n").unwrap();
    let err = PluginChannel::validate_binary(&plugin_path).unwrap_err();
    assert!(err.to_string().contains("不可执行"));
}

#[test]
fn discover_parses_json_endpoint() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-test");
    let script = r#"#!/bin/sh
if [ "$1" = "discover" ]; then
  echo '{"version":1,"type":"notify_endpoint","channel":"test","ready":true,"endpoint":{"pane":"1"},"message":"ok"}'
fi
"#;
    std::fs::write(&plugin_path, script).unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let channel = PluginChannel::new("test").unwrap();
    let endpoint = channel.discover().unwrap();
    assert_eq!(endpoint.channel, "test");
    assert!(endpoint.ready);
    assert_eq!(endpoint.endpoint, serde_json::json!({"pane": "1"}));
}

#[test]
fn discover_not_ready_on_failure() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-fail");
    std::fs::write(
        &plugin_path,
        "#!/bin/sh\nif [ \"$1\" = \"discover\" ]; then echo nope >&2; exit 1; fi\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let channel = PluginChannel::new("fail").unwrap();
    let endpoint = channel.discover().unwrap();
    assert!(!endpoint.ready);
    assert!(endpoint.message.contains("discover 失败"));
}

#[test]
fn discover_times_out_and_returns_not_ready() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-sleep");
    // discover 永远 sleep，验证 core 在 100ms 超时后返回 not_ready 且不阻塞。
    std::fs::write(
        &plugin_path,
        "#!/bin/sh\nif [ \"$1\" = \"discover\" ]; then sleep 60; fi\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // 把该插件超时设成 100ms，避免测试慢。
    let mut plugins = HashMap::new();
    plugins.insert(
        "sleep".to_string(),
        NotifyPluginEntry {
            path: plugin_path.to_string_lossy().into_owned(),
            timeout_ms: Some(100),
        },
    );
    setup_config(&tmp, plugins);

    let start = std::time::Instant::now();
    let channel = PluginChannel::new("sleep").unwrap();
    let endpoint = channel.discover().unwrap();
    let elapsed = start.elapsed();

    assert!(!endpoint.ready, "超时后应返回 not_ready");
    assert!(
        endpoint.message.contains("超时"),
        "提示应包含超时: {}",
        endpoint.message
    );
    assert!(
        elapsed < Duration::from_millis(800),
        "应在超时附近快速返回，实际耗时 {:?}",
        elapsed
    );
}

#[test]
fn send_writes_payload_to_stdin() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let out_path = tmp.path().join("out.json");
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-echo");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"send\" ]; then cat > {}; fi\n",
        out_path.to_string_lossy()
    );
    std::fs::write(&plugin_path, script).unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let channel = PluginChannel::new("echo").unwrap();
    let endpoint = serde_json::json!({"pane": "1"});
    channel.send(&endpoint, &hint(), false).unwrap();

    let written = std::fs::read_to_string(&out_path).unwrap();
    let payload: NotifyPluginSendPayload = serde_json::from_str(&written).unwrap();
    assert_eq!(payload.from_name, "nora");
    assert_eq!(payload.agent_name, "codex");
    assert_eq!(payload.type_, "notify");
    assert_eq!(payload.endpoint, endpoint);
}

#[test]
fn send_dry_run_does_not_require_real_target() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-dry");
    std::fs::write(
        &plugin_path,
        "#!/bin/sh\nif [ \"$1\" = \"send\" ] && [ \"$2\" = \"--dry-run\" ]; then exit 0; fi\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let channel = PluginChannel::new("dry").unwrap();
    let endpoint = serde_json::json!({});
    channel.send(&endpoint, &hint(), true).unwrap();
}

#[test]
fn resolve_binary_prefers_config_path() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let config_path = tmp.path().join("config-plugin");
    std::fs::write(&config_path, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut plugins = HashMap::new();
    plugins.insert(
        "pref".to_string(),
        NotifyPluginEntry {
            path: config_path.to_string_lossy().into_owned(),
            timeout_ms: None,
        },
    );
    setup_config(&tmp, plugins);

    let channel = PluginChannel::new("pref").unwrap();
    let resolved = channel.resolve_binary().unwrap();
    assert_eq!(resolved, config_path);
}

#[test]
fn resolve_binary_falls_back_to_path_prefix() {
    let tmp = TempDir::new().unwrap();
    let _guard = env_guard(&tmp);

    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let plugin_path = plugins_dir.join("agtalk-notify-fallback");
    std::fs::write(&plugin_path, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let channel = PluginChannel::new("fallback").unwrap();
    let resolved = channel.resolve_binary().unwrap();
    assert_eq!(resolved, plugin_path);
}

#[test]
fn validate_plugin_name_accepts_alphanumeric_dash_underscore() {
    assert!(validate_plugin_name("macos-notify_1").is_ok());
}

#[test]
fn validate_plugin_name_rejects_path_separator() {
    assert!(validate_plugin_name("foo/bar").is_err());
    assert!(validate_plugin_name("foo\\bar").is_err());
}

#[test]
fn validate_plugin_name_rejects_empty_and_too_long() {
    assert!(validate_plugin_name("").is_err());
    let long = "a".repeat(65);
    assert!(validate_plugin_name(&long).is_err());
}

#[test]
fn validate_plugin_name_rejects_special_chars() {
    assert!(validate_plugin_name("foo:bar").is_err());
    assert!(validate_plugin_name("foo bar").is_err());
    assert!(validate_plugin_name("foo&bar").is_err());
}
