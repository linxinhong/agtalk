//! notify 插件通道。
//!
//! 插件是用户配置的本地可执行文件，agtalk core 通过参数数组启动它（不经 shell）。
//! 插件必须支持两个子命令：
//!   - `<plugin> discover`：输出 JSON endpoint 到 stdout
//!   - `<plugin> send [--dry-run]`：从 stdin 读 JSON payload，执行提醒或只验证可用性
//!
//! core 只发"有消息"信号，不发送消息正文。

use crate::config::AgConfig;
use crate::identity::session_file::NotifyTarget;
use crate::notify::{NotifyChannel, NotifyError, NotifyHint};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_PLUGIN_TIMEOUT_MS: u64 = 1000;
const MIN_PLUGIN_TIMEOUT_MS: u64 = 100;
const MAX_PLUGIN_TIMEOUT_MS: u64 = 10000;
const MAX_PLUGIN_NAME_LEN: usize = 64;

/// 校验插件名是否合法。
/// 只允许 ASCII 字母、数字、下划线、连字符；长度 1–64。
pub fn validate_plugin_name(name: &str) -> Result<(), NotifyError> {
    if name.is_empty() {
        return Err(NotifyError::Other("插件名不能为空".to_string()));
    }
    if name.len() > MAX_PLUGIN_NAME_LEN {
        return Err(NotifyError::Other(format!(
            "插件名长度超过 {} 字符",
            MAX_PLUGIN_NAME_LEN
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(NotifyError::Other(
            "插件名只能包含 ASCII 字母、数字、下划线、连字符".to_string(),
        ));
    }
    Ok(())
}

/// 插件 discover 输出的 endpoint。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEndpoint {
    pub version: u32,
    #[serde(rename = "type")]
    pub type_: String,
    pub channel: String,
    pub ready: bool,
    #[serde(default)]
    pub endpoint: serde_json::Value,
    #[serde(default)]
    pub message: String,
}

impl PluginEndpoint {
    /// 构造一个 ready=false 的 endpoint，用于 discover 失败或插件不存在时占位。
    pub fn not_ready(channel: &str, message: impl Into<String>) -> Self {
        Self {
            version: 1,
            type_: "notify_endpoint".to_string(),
            channel: channel.to_string(),
            ready: false,
            endpoint: serde_json::Value::Null,
            message: message.into(),
        }
    }
}

/// 插件通道实现。
pub struct PluginChannel {
    name: String,
}

impl PluginChannel {
    pub fn new(name: impl Into<String>) -> Result<Self, NotifyError> {
        let name = name.into();
        validate_plugin_name(&name)?;
        Ok(Self { name })
    }

    /// 插件名。
    pub fn plugin_name(&self) -> &str {
        &self.name
    }

    /// 解析插件二进制路径：
    /// 1. 若全局配置 `notify.plugins.<name>.path` 存在，按现有规则解析；
    /// 2. 否则在 `<config_dir>/plugins/` 中查找 `agtalk-notify-<name>`；
    /// 3. 否则在 PATH 中查找 `agtalk-notify-<name>`。
    pub fn resolve_binary(&self) -> Result<PathBuf, NotifyError> {
        // 优先全局配置。
        if let Ok(config) = AgConfig::load() {
            if let Some(entry) = config.notify.plugins.get(&self.name) {
                return Self::resolve_config_path(&entry.path);
            }
        }

        let binary_name = format!("agtalk-notify-{}", self.name);

        // 默认约定目录：`<config_dir>/plugins/agtalk-notify-<name>`。
        if let Ok(plugins_dir) = crate::paths::plugins_dir() {
            let default_path = plugins_dir.join(&binary_name);
            if default_path.exists() {
                return Ok(default_path);
            }
        }

        // 回退到 PATH 中的固定前缀二进制。
        match which::which(&binary_name) {
            Ok(path) => Ok(path),
            Err(_) => Err(NotifyError::Other(format!(
                "找不到 notify 插件 '{}': 未在全局配置中定义，且 ~/.config/agtalk2/plugins/ 与 PATH 中均不存在 {}",
                self.name, binary_name
            ))),
        }
    }

    /// 解析配置中的路径：
    /// - 绝对路径按原样使用；
    /// - 相对路径或纯文件名解析为 `<config_dir>/plugins/<path>`，且禁止 `..` 逃逸。
    fn resolve_config_path(raw_path: &str) -> Result<PathBuf, NotifyError> {
        let path = Path::new(raw_path);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        // 相对路径禁止包含 .. 组件，防止逃逸出插件目录。
        for component in path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(NotifyError::Other(format!(
                    "插件相对路径禁止使用 '..': {}",
                    raw_path
                )));
            }
        }
        let plugins_dir = crate::paths::plugins_dir()
            .map_err(|e| NotifyError::Other(format!("无法定位插件目录: {}", e)))?;
        Ok(plugins_dir.join(raw_path))
    }

    /// 校验插件二进制是否可用。
    /// 要求：文件存在、是普通文件、具有可执行权限；
    /// 相对路径解析后必须位于 `<config_dir>/plugins/` 内。
    pub fn validate_binary(path: &Path) -> Result<(), NotifyError> {
        if !path.exists() {
            return Err(NotifyError::Other(format!(
                "插件路径不存在: {}",
                path.display()
            )));
        }
        let metadata = std::fs::metadata(path).map_err(NotifyError::Io)?;
        if !metadata.is_file() {
            return Err(NotifyError::Other(format!(
                "插件路径不是可执行文件: {}",
                path.display()
            )));
        }
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(NotifyError::Other(format!(
                "插件文件不可执行: {}",
                path.display()
            )));
        }
        Ok(())
    }

    /// 调用 `<plugin> discover` 获取当前 endpoint。
    /// 如果 `agent_name` 不为空，会通过环境变量 `AGTALK_NOTIFY_NAME` 传给插件，
    /// 让插件有机会把当前 pane/tab 等上下文重命名为 agent 名字。
    /// discover 与 send 一样受 `timeout_ms` 限制（默认 1000ms，100ms–10s），
    /// 避免插件 hang 住导致 CLI/daemon 无限阻塞。
    pub fn discover_with_name(
        &self,
        agent_name: Option<&str>,
    ) -> Result<PluginEndpoint, NotifyError> {
        let plugin_path = self.resolve_binary()?;
        Self::validate_binary(&plugin_path)?;

        let timeout_ms = self
            .resolve_timeout()
            .clamp(MIN_PLUGIN_TIMEOUT_MS, MAX_PLUGIN_TIMEOUT_MS);
        let timeout = Duration::from_millis(timeout_ms);

        let mut cmd = Command::new(&plugin_path);
        cmd.arg("discover");
        if let Some(name) = agent_name {
            cmd.env("AGTALK_NOTIFY_NAME", name);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| {
            NotifyError::Other(format!(
                "无法执行插件 discover {} ({}): {}",
                self.name,
                plugin_path.display(),
                e
            ))
        })?;

        match wait_with_timeout(child, timeout, &self.name) {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Ok(PluginEndpoint::not_ready(
                        &self.name,
                        format!("discover 失败: {}", stderr),
                    ));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                let endpoint: PluginEndpoint = serde_json::from_str(&stdout).map_err(|e| {
                    NotifyError::Other(format!(
                        "插件 {} discover 输出解析失败: {} (raw: {})",
                        self.name, e, stdout
                    ))
                })?;

                if endpoint.channel != self.name {
                    return Err(NotifyError::Other(format!(
                        "插件 {} discover 返回的 channel 不匹配: expected={}, got={}",
                        self.name, self.name, endpoint.channel
                    )));
                }

                Ok(endpoint)
            }
            Err(NotifyError::CommandFailed(msg)) if msg.contains("超时") => {
                Ok(PluginEndpoint::not_ready(
                    &self.name,
                    format!("discover 超时 ({}ms): 插件未在时限内响应", timeout_ms),
                ))
            }
            Err(e) => Err(e),
        }
    }

    /// 不带 agent 名字的 `discover`，用于非 join 场景（doctor、auto detect、retry 等）。
    pub fn discover(&self) -> Result<PluginEndpoint, NotifyError> {
        self.discover_with_name(None)
    }

    /// 调用 `<plugin> send [--dry-run]`。
    /// `endpoint` 是插件自定义的 JSON 对象，由 discover 输出、透传给 send。
    pub fn send(
        &self,
        endpoint: &serde_json::Value,
        hint: &NotifyHint,
        dry_run: bool,
    ) -> Result<(), NotifyError> {
        let plugin_path = self.resolve_binary()?;
        Self::validate_binary(&plugin_path)?;

        let payload = build_send_payload(endpoint, hint);
        let json = serde_json::to_vec(&payload)
            .map_err(|e| NotifyError::Other(format!("序列化插件 payload 失败: {}", e)))?;

        let timeout_ms = self
            .resolve_timeout()
            .clamp(MIN_PLUGIN_TIMEOUT_MS, MAX_PLUGIN_TIMEOUT_MS);
        let timeout = Duration::from_millis(timeout_ms);

        let mut args = vec!["send".to_string()];
        if dry_run {
            args.push("--dry-run".to_string());
        }

        let mut child = Command::new(&plugin_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                NotifyError::Other(format!(
                    "无法启动插件 {} ({}): {}",
                    self.name,
                    plugin_path.display(),
                    e
                ))
            })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| NotifyError::Other("无法获取插件 stdin".to_string()))?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(&json);
        });

        let result = wait_with_timeout(child, timeout, &self.name)?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(NotifyError::CommandFailed(format!(
                "插件 {} 返回非零退出码: {}",
                self.name, stderr
            )));
        }
        Ok(())
    }

    fn resolve_timeout(&self) -> u64 {
        if let Ok(config) = AgConfig::load() {
            if let Some(entry) = config.notify.plugins.get(&self.name) {
                return entry.timeout_ms.unwrap_or(DEFAULT_PLUGIN_TIMEOUT_MS);
            }
        }
        DEFAULT_PLUGIN_TIMEOUT_MS
    }
}

impl NotifyChannel for PluginChannel {
    fn name(&self) -> &'static str {
        "plugin"
    }

    fn discover(&self) -> Result<Option<PluginEndpoint>, NotifyError> {
        self.discover().map(Some)
    }

    fn send(
        &self,
        target: &NotifyTarget,
        hint: &NotifyHint,
        dry_run: bool,
    ) -> Result<(), NotifyError> {
        let NotifyTarget::Plugin { name, endpoint } = target else {
            return Err(NotifyError::Other(
                "插件通道需要 NotifyTarget::Plugin".to_string(),
            ));
        };
        if name != &self.name {
            return Err(NotifyError::Other(format!(
                "插件名不匹配: target={}, channel={}",
                name, self.name
            )));
        }
        self.send(endpoint, hint, dry_run)
    }
}

/// 等待子进程，带超时。超时后 kill 进程。
#[cfg(unix)]
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
    name: &str,
) -> Result<std::process::Output, NotifyError> {
    let _pid = child.id();
    let start = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(NotifyError::Io)? {
            Some(status) => {
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut o| {
                        let mut v = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut o, &mut v);
                        v
                    })
                    .unwrap_or_default();
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut o| {
                        let mut v = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut o, &mut v);
                        v
                    })
                    .unwrap_or_default();
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(NotifyError::CommandFailed(format!(
                        "插件 {} 执行超时",
                        name
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// v1 send payload，不包含消息正文与 secret。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotifyPluginSendPayload {
    pub version: u32,
    #[serde(rename = "type")]
    pub type_: String,
    pub endpoint: serde_json::Value,
    pub from_name: String,
    /// 人类可读的取信命令，带 `--as <agent_name>`。
    pub read_command: String,
    /// 插件可直接执行的安全参数数组。
    pub read_args: Vec<String>,
    pub binary_path: String,
    pub agent_name: String,
    pub agent_address: String,
    /// 触发本次 notify 的消息 ID。
    pub message_id: String,
    /// 插件应直接注入终端的完整文本，由 daemon 统一组装。
    pub text: String,
    /// 注入后是否自动发送 Enter 执行命令。
    pub send_enter: bool,
}

pub(crate) fn short_id(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_string()
}

fn build_send_payload(endpoint: &serde_json::Value, hint: &NotifyHint) -> NotifyPluginSendPayload {
    let read_args = vec![
        "--as".to_string(),
        hint.agent_name.clone(),
        "msg".to_string(),
        "read".to_string(),
    ];
    let read_command = format!("agtalk --as {} msg read", hint.agent_name);
    let text = crate::notify::build_hint_text(hint);
    NotifyPluginSendPayload {
        version: 1,
        type_: "notify".to_string(),
        endpoint: endpoint.clone(),
        from_name: hint.from_name.clone(),
        read_command,
        read_args,
        binary_path: hint.binary_path.clone(),
        agent_name: hint.agent_name.clone(),
        agent_address: hint.agent_address.clone(),
        message_id: hint.message_id.clone(),
        text,
        send_enter: hint.send_enter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgConfig, NotifyConfig, NotifyPluginEntry};
    use crate::notify::NotifyHint;
    use crate::paths::CONFIG_DIR_ENV;
    use std::collections::HashMap;
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
}
