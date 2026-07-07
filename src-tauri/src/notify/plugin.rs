//! notify 插件通道。
//!
//! 插件是用户配置的本地可执行文件，daemon 通过参数数组启动它（不经 shell），
//! 并向 stdin 写入 JSON payload。插件只收到“有消息”信号，不收到消息正文。

use crate::config::{AgConfig, NotifyPluginEntry};
use crate::identity::session_file::NotifyTarget;
use crate::notify::{NotifyChannel, NotifyError, NotifyHint};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// 插件通道实现。
pub struct PluginChannel {
    name: String,
}

impl PluginChannel {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// 从全局配置读取插件定义。
    pub fn resolve_entry(&self) -> Result<NotifyPluginEntry, NotifyError> {
        let config =
            AgConfig::load().map_err(|e| NotifyError::Other(format!("无法加载全局配置: {}", e)))?;
        let entry = config
            .notify
            .plugins
            .get(&self.name)
            .ok_or_else(|| {
                NotifyError::Other(format!("notify 插件 '{}' 未在全局配置中定义", self.name))
            })?
            .clone();
        Ok(entry)
    }

    /// 解析插件路径：
    /// - 绝对路径按原样使用；
    /// - 相对路径或纯文件名解析为 `<config_dir>/plugins/<path>`。
    pub fn resolve_plugin_path(raw_path: &str) -> Result<PathBuf, NotifyError> {
        let path = Path::new(raw_path);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        let plugins_dir = crate::paths::plugins_dir()
            .map_err(|e| NotifyError::Other(format!("无法定位插件目录: {}", e)))?;
        Ok(plugins_dir.join(raw_path))
    }

    /// 校验插件路径是否可用。
    pub fn validate_entry(entry: &NotifyPluginEntry) -> Result<PathBuf, NotifyError> {
        let path = Self::resolve_plugin_path(&entry.path)?;
        if !path.exists() {
            return Err(NotifyError::Other(format!(
                "插件路径不存在: {}",
                path.display()
            )));
        }
        let metadata = std::fs::metadata(&path).map_err(NotifyError::Io)?;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(NotifyError::Other(format!(
                "插件文件不可执行: {}",
                path.display()
            )));
        }
        Ok(path)
    }
}

impl NotifyChannel for PluginChannel {
    fn name(&self) -> &'static str {
        "plugin"
    }

    fn inject(&self, target: &NotifyTarget, hint: &NotifyHint) -> Result<(), NotifyError> {
        let NotifyTarget::Plugin { name } = target else {
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

        let entry = self.resolve_entry()?;
        let plugin_path = Self::validate_entry(&entry)?;

        let payload = build_payload(hint);
        let json = serde_json::to_vec(&payload)
            .map_err(|e| NotifyError::Other(format!("序列化插件 payload 失败: {}", e)))?;

        let timeout_ms = entry.timeout_ms.unwrap_or(1000);
        let timeout = Duration::from_millis(timeout_ms);

        let mut child = Command::new(&plugin_path)
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

        let result = wait_with_timeout(child, timeout)?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(NotifyError::CommandFailed(format!(
                "插件 {} 返回非零退出码: {}",
                self.name, stderr
            )));
        }
        Ok(())
    }
}

/// 等待子进程，带超时。超时后 kill 进程。
#[cfg(unix)]
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, NotifyError> {
    let pid = child.id();
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
                    return Err(NotifyError::CommandFailed(format!("插件 {} 执行超时", pid)));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// v1 payload，不包含消息正文与 secret。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NotifyPluginPayload {
    pub version: u32,
    #[serde(rename = "type")]
    pub type_: String,
    pub from_name: String,
    pub read_command: String,
    pub binary_path: String,
    pub workspace: String,
    pub agent_name: String,
    pub agent_address: String,
}

fn build_payload(hint: &NotifyHint) -> NotifyPluginPayload {
    NotifyPluginPayload {
        version: 1,
        type_: "notify".to_string(),
        from_name: hint.from_name.clone(),
        read_command: format!("{} msg read", hint.binary_path),
        binary_path: hint.binary_path.clone(),
        workspace: hint.workspace.clone(),
        agent_name: hint.agent_name.clone(),
        agent_address: hint.agent_address.clone(),
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
            workspace: "projA".to_string(),
            agent_name: "codex".to_string(),
            agent_address: "550e8400-e29b-41d4-a716-446655440000".to_string(),
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

    #[test]
    fn payload_does_not_include_body_or_secret() {
        let payload = build_payload(&hint());
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("nora"));
        assert!(json.contains("/usr/local/bin/agtalk msg read"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("message body"));
    }

    #[test]
    fn resolve_plugin_path_uses_absolute_as_is() {
        let path = PluginChannel::resolve_plugin_path("/usr/bin/plugin").unwrap();
        assert_eq!(path, PathBuf::from("/usr/bin/plugin"));
    }

    #[test]
    fn resolve_plugin_path_resolves_filename_to_plugins_dir() {
        let tmp = TempDir::new().unwrap();
        let prev = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());

        let path = PluginChannel::resolve_plugin_path("my-plugin").unwrap();
        assert_eq!(path, tmp.path().join("plugins").join("my-plugin"));

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }

    #[test]
    fn resolve_plugin_path_resolves_relative_to_plugins_dir() {
        let tmp = TempDir::new().unwrap();
        let prev = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());

        let path = PluginChannel::resolve_plugin_path("subdir/my-plugin").unwrap();
        assert_eq!(path, tmp.path().join("plugins").join("subdir/my-plugin"));

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }

    #[test]
    fn validate_rejects_missing_file() {
        let entry = NotifyPluginEntry {
            path: "/nonexistent/plugin".to_string(),
            timeout_ms: None,
        };
        let err = PluginChannel::validate_entry(&entry).unwrap_err();
        assert!(err.to_string().contains("不存在"));
    }

    #[test]
    fn plugin_success_writes_payload_to_stdin() {
        let tmp = TempDir::new().unwrap();
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());
        let prev = std::env::var_os(CONFIG_DIR_ENV);

        let out_path = tmp.path().join("out.json");
        let script = format!("#!/bin/sh\ncat > {}\n", out_path.to_string_lossy());
        let plugin_path = tmp.path().join("fake-plugin.sh");
        std::fs::write(&plugin_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut plugins = HashMap::new();
        plugins.insert(
            "test".to_string(),
            NotifyPluginEntry {
                path: plugin_path.to_string_lossy().into_owned(),
                timeout_ms: Some(1000),
            },
        );
        setup_config(&tmp, plugins);

        let channel = PluginChannel::new("test");
        let target = NotifyTarget::Plugin {
            name: "test".to_string(),
        };
        channel.inject(&target, &hint()).unwrap();

        let written = std::fs::read_to_string(&out_path).unwrap();
        let payload: NotifyPluginPayload = serde_json::from_str(&written).unwrap();
        assert_eq!(payload.from_name, "nora");
        assert_eq!(payload.agent_name, "codex");
        assert_eq!(payload.type_, "notify");

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }

    #[test]
    fn plugin_success_with_filename_resolves_to_plugins_dir() {
        let tmp = TempDir::new().unwrap();
        let prev = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());

        let plugins_dir = tmp.path().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();

        let out_path = tmp.path().join("out.json");
        let script = format!("#!/bin/sh\ncat > {}\n", out_path.to_string_lossy());
        let plugin_path = plugins_dir.join("fake-plugin.sh");
        std::fs::write(&plugin_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut plugins = HashMap::new();
        plugins.insert(
            "test".to_string(),
            NotifyPluginEntry {
                path: "fake-plugin.sh".to_string(),
                timeout_ms: Some(1000),
            },
        );
        setup_config(&tmp, plugins);

        let channel = PluginChannel::new("test");
        let target = NotifyTarget::Plugin {
            name: "test".to_string(),
        };
        channel.inject(&target, &hint()).unwrap();

        let written = std::fs::read_to_string(&out_path).unwrap();
        let payload: NotifyPluginPayload = serde_json::from_str(&written).unwrap();
        assert_eq!(payload.from_name, "nora");

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }

    #[test]
    fn plugin_failure_returns_error() {
        let tmp = TempDir::new().unwrap();
        let prev = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());

        let plugin_path = tmp.path().join("fail-plugin.sh");
        std::fs::write(&plugin_path, "#!/bin/sh\necho error >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut plugins = HashMap::new();
        plugins.insert(
            "fail".to_string(),
            NotifyPluginEntry {
                path: plugin_path.to_string_lossy().into_owned(),
                timeout_ms: Some(1000),
            },
        );
        setup_config(&tmp, plugins);

        let channel = PluginChannel::new("fail");
        let target = NotifyTarget::Plugin {
            name: "fail".to_string(),
        };
        let err = channel.inject(&target, &hint()).unwrap_err();
        assert!(err.to_string().contains("非零退出码"));

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }

    #[test]
    fn plugin_timeout_kills_process() {
        let tmp = TempDir::new().unwrap();
        let prev = std::env::var_os(CONFIG_DIR_ENV);
        std::env::set_var(CONFIG_DIR_ENV, tmp.path());

        let plugin_path = tmp.path().join("slow-plugin.sh");
        std::fs::write(&plugin_path, "#!/bin/sh\nsleep 10\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plugin_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut plugins = HashMap::new();
        plugins.insert(
            "slow".to_string(),
            NotifyPluginEntry {
                path: plugin_path.to_string_lossy().into_owned(),
                timeout_ms: Some(50),
            },
        );
        setup_config(&tmp, plugins);

        let channel = PluginChannel::new("slow");
        let target = NotifyTarget::Plugin {
            name: "slow".to_string(),
        };
        let err = channel.inject(&target, &hint()).unwrap_err();
        assert!(err.to_string().contains("超时"));

        if let Some(v) = prev {
            std::env::set_var(CONFIG_DIR_ENV, v);
        } else {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
    }
}
