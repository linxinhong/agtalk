//! doctor 系统/配置/飞书/daemon 检查（doctor.rs 拆分，控制行数红线）。
use crate::config::AgConfig;
use crate::proto::DiagnosisCheck;
use crate::tool::doctor::*;
use crate::tool::DoctorContext;
use std::time::Duration;
use sysinfo::{Pid, System};

pub(crate) fn runtime_checks() -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let binary = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    checks.push(check(
        "runtime",
        "runtime.binary",
        "ok",
        &binary,
        None,
        None,
        serde_json::Value::Null,
    ));

    checks.push(check(
        "runtime",
        "runtime.version",
        "ok",
        env!("CARGO_PKG_VERSION"),
        None,
        None,
        serde_json::Value::Null,
    ));

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    checks.push(check(
        "runtime",
        "runtime.cwd",
        "ok",
        &cwd,
        None,
        None,
        serde_json::Value::Null,
    ));

    let config_dir = crate::paths::config_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    checks.push(check(
        "runtime",
        "runtime.config_dir",
        "ok",
        &config_dir,
        None,
        None,
        serde_json::Value::Null,
    ));

    checks
}

// ---- config ----

pub(crate) fn config_checks(config: &AgConfig) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let path = AgConfig::path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    checks.push(check(
        "config",
        "config.path",
        "ok",
        &path,
        None,
        None,
        serde_json::json!({ "path": path }),
    ));

    checks.push(check(
        "config",
        "config.http_port",
        "ok",
        format!("http_port = {}", config.http_port),
        None,
        None,
        serde_json::json!({ "http_port": config.http_port }),
    ));

    checks
}

// ---- feishu ----

/// 飞书 human transport 检查：仅检查本地配置（长连接状态见 daemon 日志）。
pub(crate) fn feishu_checks(config: &AgConfig) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();
    let f = &config.feishu;

    if !f.enabled {
        checks.push(check(
            "feishu",
            "feishu.enabled",
            "ok",
            "feishu transport not enabled",
            None,
            None,
            serde_json::json!({ "enabled": false }),
        ));
        return checks;
    }

    checks.push(check(
        "feishu",
        "feishu.enabled",
        "ok",
        "feishu transport enabled",
        None,
        None,
        serde_json::json!({ "enabled": true, "base_url": f.base_url }),
    ));

    checks.push(credential_check(
        "feishu.app_id",
        &f.app_id,
        "app_id",
        "agtalk config set feishu.app_id <app_id>",
    ));
    checks.push(credential_check(
        "feishu.app_secret",
        &f.app_secret,
        "app_secret",
        "agtalk config set feishu.app_secret <app_secret>",
    ));

    if f.open_id.is_empty() {
        checks.push(check(
            "feishu",
            "feishu.open_id",
            "warn",
            "open_id not bound: card clicks and messages from feishu will be ignored",
            Some("bind the human user's open_id so feishu events are accepted"),
            Some("agtalk config set feishu.open_id <open_id>"),
            serde_json::json!({ "open_id": "" }),
        ));
    } else {
        checks.push(check(
            "feishu",
            "feishu.open_id",
            "ok",
            format!("open_id bound ({})", mask(&f.open_id)),
            None,
            None,
            serde_json::json!({ "open_id": mask(&f.open_id) }),
        ));
    }

    if !config.human.surfaces.iter().any(|s| s == "feishu") {
        checks.push(check(
            "feishu",
            "feishu.surface",
            "warn",
            "human.surfaces does not include feishu: messages will not be delivered to feishu",
            Some("add feishu to human.surfaces to enable delivery"),
            Some("agtalk config set human.surfaces '[\"popup\",\"feishu\"]'"),
            serde_json::json!({ "surfaces": config.human.surfaces }),
        ));
    }

    checks
}

pub(crate) fn credential_check(
    name: &str,
    value: &str,
    label: &str,
    command: &str,
) -> DiagnosisCheck {
    if value.is_empty() {
        check(
            "feishu",
            name,
            "error",
            format!("feishu {} is empty", label),
            Some("feishu is enabled but credentials are incomplete"),
            Some(command),
            serde_json::json!({}),
        )
    } else {
        check(
            "feishu",
            name,
            "ok",
            format!("{} configured ({})", label, mask(value)),
            None,
            None,
            serde_json::json!({}),
        )
    }
}

/// 脱敏：只保留前 4 位。
pub(crate) fn mask(value: &str) -> String {
    let prefix: String = value.chars().take(4).collect();
    format!("{}***", prefix)
}

// ---- daemon ----

pub(crate) fn daemon_checks(ctx: &DoctorContext) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();
    let port = ctx.config.http_port;
    let daemon_running = crate::cli::daemon::is_running();

    let status_file = crate::server::daemon::read_status_file();
    match status_file {
        Ok(Some(file)) => {
            checks.push(check(
                "daemon",
                "daemon.status_file",
                "ok",
                format!("daemon.json 存在 (pid {})", file.pid),
                None,
                None,
                serde_json::json!({ "pid": file.pid, "http_port": file.http_port }),
            ));

            let mut sys = System::new_all();
            sys.refresh_processes();
            let pid_alive = sys.process(Pid::from(file.pid as usize)).is_some();
            checks.push(check(
                "daemon",
                "daemon.pid",
                if pid_alive { "ok" } else { "error" },
                if pid_alive {
                    format!("pid {} 存活", file.pid)
                } else {
                    format!("pid {} 已死亡，状态文件残留", file.pid)
                },
                if pid_alive {
                    None
                } else {
                    Some("运行 `agtalk daemon restart` 清理并重启")
                },
                if pid_alive {
                    None
                } else {
                    Some("agtalk daemon restart")
                },
                serde_json::json!({ "pid": file.pid }),
            ));

            let http_url = format!("http://127.0.0.1:{}/api/v1/daemon/status", file.http_port);
            match http_get(&http_url, &[]) {
                Ok(body) => {
                    let version_match = body
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|v| v == env!("CARGO_PKG_VERSION"))
                        .unwrap_or(false);
                    checks.push(check(
                        "daemon",
                        "daemon.http",
                        "ok",
                        format!("HTTP 可达: {}", http_url),
                        None,
                        None,
                        serde_json::json!({ "url": http_url }),
                    ));
                    checks.push(check(
                        "daemon",
                        "daemon.version",
                        if version_match { "ok" } else { "warn" },
                        if version_match {
                            "CLI 与 daemon 版本一致".to_string()
                        } else {
                            "CLI 与 daemon 版本不一致".to_string()
                        },
                        if version_match {
                            None
                        } else {
                            Some("建议重启 daemon 使版本一致")
                        },
                        if version_match {
                            None
                        } else {
                            Some("agtalk daemon restart")
                        },
                        serde_json::Value::Null,
                    ));
                }
                Err(e) => {
                    checks.push(check(
                        "daemon",
                        "daemon.http",
                        "error",
                        format!("HTTP 不可达: {}", e),
                        Some("检查 daemon 是否已挂或无响应"),
                        Some("agtalk daemon restart"),
                        serde_json::json!({ "url": http_url }),
                    ));
                    checks.push(check(
                        "daemon",
                        "daemon.version",
                        "skip",
                        "daemon HTTP 不可达，跳过版本检查",
                        None,
                        None,
                        serde_json::Value::Null,
                    ));
                }
            }
        }
        Ok(None) => {
            checks.push(check(
                "daemon",
                "daemon.status_file",
                "error",
                "daemon.json 不存在，daemon 未运行",
                Some("运行 `agtalk daemon start` 启动 daemon"),
                Some("agtalk daemon start"),
                serde_json::Value::Null,
            ));
            checks.push(check(
                "daemon",
                "daemon.pid",
                "skip",
                "daemon 未运行，跳过 pid 检查",
                None,
                None,
                serde_json::Value::Null,
            ));
            checks.push(check(
                "daemon",
                "daemon.http",
                "skip",
                "daemon 未运行，跳过 HTTP 检查",
                None,
                None,
                serde_json::Value::Null,
            ));
            checks.push(check(
                "daemon",
                "daemon.version",
                "skip",
                "daemon 未运行，跳过版本检查",
                None,
                None,
                serde_json::Value::Null,
            ));
        }
        Err(e) => {
            checks.push(check(
                "daemon",
                "daemon.status_file",
                "warn",
                format!("读取 daemon.json 失败: {}", e),
                Some("检查 ~/.config/agtalk2 目录权限"),
                None,
                serde_json::Value::Null,
            ));
        }
    }

    let port_in_use = std::net::TcpListener::bind(("127.0.0.1", port)).is_err();
    let port_status = if port_in_use {
        if daemon_running {
            "ok"
        } else {
            "warn"
        }
    } else {
        "ok"
    };
    checks.push(check(
        "daemon",
        "daemon.port",
        port_status,
        if port_in_use {
            format!("端口 {} 已被占用", port)
        } else {
            format!("端口 {} 可用", port)
        },
        if port_in_use && !daemon_running {
            Some("若 daemon 未运行，检查是否有其他进程占用该端口")
        } else {
            None
        },
        None,
        serde_json::json!({ "port": port }),
    ));

    checks
}

// ---- identity ----

#[derive(Debug, Clone)]
pub(crate) struct ResolvedIdentity {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) session_path: std::path::PathBuf,
    pub(crate) source: String, // env | as | ancestor | single_session | none
}

pub(crate) fn http_get(url: &str, headers: &[(&str, String)]) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, v.clone());
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json().map_err(|e| e.to_string())
}

pub(crate) fn http_get_stream_head(url: &str, headers: &[(&str, String)]) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, v.clone());
    }
    let resp = req.send().map_err(|e| e.to_string())?;
    if resp.status().is_success() || resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // 401 说明 SSE 端点活着，只是认证失败（我们没传 start_time）
        return Ok(());
    }
    Err(format!("HTTP {}", resp.status()))
}
