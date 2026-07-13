//! `agtalk tool doctor` 实现。

use crate::config::AgConfig;
use crate::identity::agents_map;
use crate::identity::mailbox;
use crate::identity::relations;
use crate::identity::session_file::{self, NotifyTarget};
use crate::notify;
use crate::notify::NotifyHint;
use crate::proto::{DiagnosisCheck, RootCause, ServerMsg};
use crate::routing::inbox;
use crate::tool::DoctorContext;
use std::path::Path;
use std::time::Duration;
use sysinfo::{Pid, System};

/// 运行 doctor 检查。
pub fn run(ctx: DoctorContext) -> ServerMsg {
    let mut checks = Vec::new();

    checks.extend(runtime_checks());
    checks.extend(config_checks(&ctx.config));
    checks.extend(feishu_checks(&ctx.config));
    checks.extend(daemon_checks(&ctx));

    let identity = resolve_identity_readonly(&ctx);
    checks.extend(identity_checks(&ctx, &identity));
    checks.extend(local_state_checks(&ctx, &identity));
    checks.extend(message_checks(&ctx, &identity));
    checks.extend(wait_checks(&ctx, &identity));
    checks.extend(notify_checks(&ctx, &identity));

    let status = aggregate_status(&checks);
    let summary = compute_summary(&status, &checks);
    let root_causes = compute_root_causes(&checks, &identity);
    let actions = compute_actions(&root_causes, &identity);
    let context = compute_context(&identity, &checks);

    ServerMsg::ToolDiagnosis {
        status,
        summary,
        root_causes,
        actions,
        context,
        checks,
    }
}

fn aggregate_status(checks: &[DiagnosisCheck]) -> String {
    if checks.iter().any(|c| c.status == "error") {
        "error".to_string()
    } else if checks.iter().any(|c| c.status == "warn") {
        "warn".to_string()
    } else {
        "ok".to_string()
    }
}

fn compute_summary(status: &str, checks: &[DiagnosisCheck]) -> String {
    match status {
        "error" => {
            if has_error(checks, "daemon.status_file") {
                "local agent bus unavailable".to_string()
            } else if has_error(checks, "identity.db_mailbox")
                && has_error(checks, "message.mailbox")
            {
                "session exists but mailbox missing in DB".to_string()
            } else {
                "agent environment has errors".to_string()
            }
        }
        "warn" => "agent environment has warnings".to_string(),
        _ => "agent environment is healthy".to_string(),
    }
}

fn has_error(checks: &[DiagnosisCheck], name: &str) -> bool {
    checks.iter().any(|c| c.name == name && c.status == "error")
}

fn compute_root_causes(
    checks: &[DiagnosisCheck],
    identity: &Option<ResolvedIdentity>,
) -> Vec<RootCause> {
    let mut causes = Vec::new();
    let mut stale_db = false;
    let mut stale_msg = false;

    for c in checks {
        if c.status != "error" && c.status != "warn" {
            continue;
        }
        match c.name.as_str() {
            "daemon.status_file" => {
                causes.push(RootCause {
                    id: "daemon.stopped".to_string(),
                    status: "error".to_string(),
                    message: c.message.clone(),
                    command: Some("agtalk daemon start".to_string()),
                });
            }
            "identity.db_mailbox" => stale_db = true,
            "message.mailbox" => stale_msg = true,
            _ => {
                causes.push(RootCause {
                    id: c.name.clone(),
                    status: c.status.clone(),
                    message: c.message.clone(),
                    command: c.command.clone(),
                });
            }
        }
    }

    if stale_db && stale_msg {
        let name = identity
            .as_ref()
            .map(|id| id.name.clone())
            .unwrap_or_default();
        causes.push(RootCause {
            id: "identity.stale_mailbox".to_string(),
            status: "error".to_string(),
            message: format!("session {} 存在，但 DB 中无 mailbox", name),
            command: Some(format!("agtalk id join {}", name)),
        });
    }

    // daemon stopped 时，message/wait/notify 中的派生错误不是独立根因；
    // 保留 daemon.* 与 identity.*（含合并后的 identity.stale_mailbox）。
    if causes.iter().any(|c| c.id == "daemon.stopped") {
        causes.retain(|c| c.id == "daemon.stopped" || c.id.starts_with("identity."));
    }

    causes
}

fn compute_actions(root_causes: &[RootCause], identity: &Option<ResolvedIdentity>) -> Vec<String> {
    let mut actions = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cause in root_causes {
        let cmd = match cause.id.as_str() {
            "daemon.stopped" => Some("agtalk daemon start".to_string()),
            "identity.stale_mailbox" => {
                let name = identity
                    .as_ref()
                    .map(|id| id.name.clone())
                    .unwrap_or_default();
                Some(format!("agtalk id join {}", name))
            }
            _ => cause.command.clone(),
        };
        if let Some(cmd) = cmd {
            if seen.insert(cmd.clone()) {
                actions.push(cmd);
            }
        }
    }

    actions
}

fn compute_context(
    identity: &Option<ResolvedIdentity>,
    checks: &[DiagnosisCheck],
) -> crate::proto::DiagnosisContext {
    let pending = identity.as_ref().and_then(|_id| {
        checks
            .iter()
            .find(|c| c.name == "message.pending")
            .and_then(|c| c.details.get("pending"))
            .and_then(|v| v.as_i64())
    });

    crate::proto::DiagnosisContext {
        identity: identity.as_ref().map(|id| id.name.clone()),
        address: identity.as_ref().map(|id| id.address.clone()),
        pending,
    }
}

fn check(
    category: &str,
    name: &str,
    status: &str,
    message: impl Into<String>,
    suggestion: Option<&str>,
    command: Option<&str>,
    details: serde_json::Value,
) -> DiagnosisCheck {
    DiagnosisCheck {
        category: category.to_string(),
        name: name.to_string(),
        status: status.to_string(),
        message: message.into(),
        suggestion: suggestion.map(|s| s.to_string()),
        command: command.map(|s| s.to_string()),
        details,
    }
}

// ---- runtime ----

fn runtime_checks() -> Vec<DiagnosisCheck> {
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

fn config_checks(config: &AgConfig) -> Vec<DiagnosisCheck> {
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
fn feishu_checks(config: &AgConfig) -> Vec<DiagnosisCheck> {
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

fn credential_check(name: &str, value: &str, label: &str, command: &str) -> DiagnosisCheck {
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
fn mask(value: &str) -> String {
    let prefix: String = value.chars().take(4).collect();
    format!("{}***", prefix)
}

// ---- daemon ----

fn daemon_checks(ctx: &DoctorContext) -> Vec<DiagnosisCheck> {
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
struct ResolvedIdentity {
    name: String,
    address: String,
    session_path: std::path::PathBuf,
    source: String, // env | as | ancestor | single_session | none
}

fn resolve_identity_readonly(ctx: &DoctorContext) -> Option<ResolvedIdentity> {
    let dot = &ctx.dot_agtalk;

    // 1. --as / AGTALK_NAME
    let selected_name: Option<String> = ctx
        .as_name
        .clone()
        .or_else(|| std::env::var("AGTALK_NAME").ok());
    if let Some(name) = selected_name {
        let path = dot.join(&name).join("session.json");
        if let Ok(session) = session_file::read(dot, &name) {
            return Some(ResolvedIdentity {
                name: session.name,
                address: session.address,
                session_path: path,
                source: "env_or_as".to_string(),
            });
        }
        return None;
    }

    // 2. 已注册祖先 PID（只读）
    if let Ok(Some(entry)) = find_registered_ancestor(dot) {
        let path = dot.join(&entry.name).join("session.json");
        if let Ok(session) = session_file::read(dot, &entry.name) {
            return Some(ResolvedIdentity {
                name: session.name,
                address: session.address,
                session_path: path,
                source: "ancestor".to_string(),
            });
        }
    }

    // 3. 单 session 自动恢复（只读，不注册）
    if let Ok(names) = list_session_names(dot) {
        if names.len() == 1 {
            let name = &names[0];
            let path = dot.join(name).join("session.json");
            if let Ok(session) = session_file::read(dot, name) {
                return Some(ResolvedIdentity {
                    name: session.name,
                    address: session.address,
                    session_path: path,
                    source: "single_session".to_string(),
                });
            }
        }
    }

    None
}

fn find_registered_ancestor(dot: &Path) -> Result<Option<agents_map::AgentEntry>, String> {
    let map = agents_map::read(dot).map_err(|e| e.to_string())?;
    let mut sys = System::new_all();
    sys.refresh_processes();

    let mut pid = std::process::id();

    if let Some(entry) = map.anchors.get(&pid.to_string()) {
        let st = os_process_start_time(&mut sys, pid);
        if st != 0 && st == entry.start_time {
            return Ok(Some(entry.clone()));
        }
    }

    while let Some(process) = sys.process(Pid::from(pid as usize)) {
        let parent = match process.parent() {
            Some(p) => p,
            None => break,
        };
        let ppid = parent.as_u32();
        if ppid == 0 || ppid == pid {
            break;
        }
        if let Some(entry) = map.anchors.get(&ppid.to_string()) {
            let st = os_process_start_time(&mut sys, ppid);
            if st != 0 && st == entry.start_time {
                return Ok(Some(entry.clone()));
            }
        }
        pid = ppid;
    }

    Ok(None)
}

fn os_process_start_time(sys: &mut System, pid: u32) -> u64 {
    sys.refresh_processes();
    sys.process(Pid::from(pid as usize))
        .map(|p| p.start_time())
        .unwrap_or(0)
}

fn list_session_names(dot: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if !dot.exists() {
        return Ok(names);
    }
    for entry in std::fs::read_dir(dot).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let session_path = entry.path().join("session.json");
        if session_path.exists() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

fn identity_checks(
    ctx: &DoctorContext,
    identity: &Option<ResolvedIdentity>,
) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();
    let dot = &ctx.dot_agtalk;

    let dot_exists = dot.exists();
    checks.push(check(
        "identity",
        "identity.workspace_root",
        "ok",
        format!("当前目录 identity root: {}", dot.display()),
        None,
        None,
        serde_json::json!({ "workspace_root": dot }),
    ));
    checks.push(check(
        "identity",
        "identity.dot_agtalk",
        if dot_exists { "ok" } else { "warn" },
        if dot_exists {
            format!(".agtalk/ 存在: {}", dot.display())
        } else {
            ".agtalk/ 不存在".to_string()
        },
        if dot_exists {
            None
        } else {
            Some("执行 `agtalk id join <name>` 会创建该目录")
        },
        if dot_exists {
            None
        } else {
            Some("agtalk id join <name>")
        },
        serde_json::Value::Null,
    ));

    let sessions = list_session_names(dot).unwrap_or_default();
    checks.push(check(
        "identity",
        "identity.sessions",
        "ok",
        format!("发现 {} 个 session", sessions.len()),
        None,
        None,
        serde_json::json!({ "sessions": sessions }),
    ));

    let env_name = std::env::var("AGTALK_NAME").ok();
    checks.push(check(
        "identity",
        "identity.env_name",
        if env_name.is_some() { "ok" } else { "info" },
        if let Some(name) = &env_name {
            format!("AGTALK_NAME={}", name)
        } else {
            "未设置 AGTALK_NAME".to_string()
        },
        None,
        None,
        serde_json::Value::Null,
    ));

    if dot_exists {
        let ancestor = find_registered_ancestor(dot).ok().flatten();
        checks.push(check(
            "identity",
            "identity.pid_anchor",
            if ancestor.is_some() { "ok" } else { "info" },
            if let Some(entry) = &ancestor {
                format!("找到已注册祖先 pid: {}, name: {}", entry.name, entry.name)
            } else {
                "未找到已注册祖先 pid".to_string()
            },
            if ancestor.is_some() {
                None
            } else {
                Some("当前 shell 可能未执行过 agtalk join")
            },
            None,
            serde_json::Value::Null,
        ));
    } else {
        checks.push(check(
            "identity",
            "identity.pid_anchor",
            "skip",
            ".agtalk/ 不存在，跳过祖先检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    let Some(id) = identity else {
        checks.push(check(
            "identity",
            "identity.session_file",
            "skip",
            "身份未解析，跳过 session 文件检查",
            Some("使用 --as <name>、AGTALK_NAME 或先执行 agtalk id join"),
            Some("agtalk id join <name>"),
            serde_json::Value::Null,
        ));
        checks.push(check(
            "identity",
            "identity.address",
            "skip",
            "身份未解析，跳过 address 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "identity",
            "identity.db_mailbox",
            "skip",
            "身份未解析，跳过 DB mailbox 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "identity",
            "identity.permissions",
            "skip",
            "身份未解析，跳过权限检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        return checks;
    };

    checks.push(check(
        "identity",
        "identity.session_file",
        "ok",
        format!("session 文件存在: {}", id.session_path.display()),
        None,
        None,
        serde_json::json!({ "source": id.source, "name": id.name }),
    ));

    let address_ok = uuid::Uuid::parse_str(&id.address).is_ok();
    checks.push(check(
        "identity",
        "identity.address",
        if address_ok { "ok" } else { "error" },
        if address_ok {
            format!("address 是有效 UUID: {}", id.address)
        } else {
            format!("address 不是有效 UUID: {}", id.address)
        },
        if address_ok {
            None
        } else {
            Some("session.json 损坏，建议重新 join")
        },
        if address_ok {
            None
        } else {
            Some("agtalk id leave --purge && agtalk id join <name>")
        },
        serde_json::json!({ "address": id.address }),
    ));

    let mb_exists = ctx
        .storage
        .as_ref()
        .and_then(|s| mailbox::get_by_address(s, &id.address).ok())
        .flatten()
        .is_some();
    checks.push(check(
        "identity",
        "identity.db_mailbox",
        if mb_exists { "ok" } else { "error" },
        if mb_exists {
            "address 存在于 daemon DB".to_string()
        } else {
            "address 不存在于 daemon DB".to_string()
        },
        if mb_exists {
            None
        } else {
            Some("session 有效但 DB 中无对应 mailbox，建议重新 join")
        },
        if mb_exists {
            None
        } else {
            Some("agtalk id join <name>")
        },
        serde_json::json!({ "address": id.address }),
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm_ok = std::fs::metadata(&id.session_path)
            .map(|m| m.permissions().mode() & 0o777 == 0o600)
            .unwrap_or(false);
        checks.push(check(
            "identity",
            "identity.permissions",
            if perm_ok { "ok" } else { "warn" },
            if perm_ok {
                "session.json 权限为 0600".to_string()
            } else {
                "session.json 权限不是 0600".to_string()
            },
            if perm_ok {
                None
            } else {
                Some("建议手动修复权限: chmod 600 session.json")
            },
            None,
            serde_json::Value::Null,
        ));
    }
    #[cfg(not(unix))]
    {
        checks.push(check(
            "identity",
            "identity.permissions",
            "skip",
            "非 Unix 平台，跳过权限检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    checks
}

/// 检查已解析身份对应 agent 目录下的 history.jsonl / relations.json 状态，
/// 暴露 history 写入失败、权限错误、relations owner 与身份不一致等问题。
/// 身份未解析时返回空（由 identity_checks 的 skip 路径覆盖）。
fn local_state_checks(
    ctx: &DoctorContext,
    identity: &Option<ResolvedIdentity>,
) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();
    let Some(id) = identity else {
        return checks;
    };

    let dot = &ctx.dot_agtalk;
    let agent_dir = match id.session_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return checks,
    };

    // agent 目录可写性（append_jsonl 需要 owner 可写，否则 history 会静默失败）
    let dir_writable = path_mode(&agent_dir)
        .map(|m| m & 0o200 != 0)
        .unwrap_or(false);
    checks.push(check(
        "identity",
        "identity.agent_dir_writable",
        if dir_writable { "ok" } else { "warn" },
        if dir_writable {
            format!("agent 目录可写: {}", agent_dir.display())
        } else {
            format!("agent 目录不可写或不存在: {}", agent_dir.display())
        },
        if dir_writable {
            None
        } else {
            Some("history/relations 写入会静默失败，请检查目录权限")
        },
        None,
        serde_json::json!({ "dir": agent_dir.display().to_string() }),
    ));

    // history.jsonl：存在则校验 0600
    let history_path = agent_dir.join("history.jsonl");
    if history_path.exists() {
        let perm_ok = path_mode(&history_path) == Some(0o600);
        checks.push(check(
            "identity",
            "identity.history_file",
            if perm_ok { "ok" } else { "warn" },
            if perm_ok {
                "history.jsonl 权限为 0600".to_string()
            } else {
                "history.jsonl 权限不是 0600".to_string()
            },
            if perm_ok {
                None
            } else {
                Some("建议: chmod 600 history.jsonl")
            },
            None,
            serde_json::json!({ "path": history_path.display().to_string() }),
        ));
    }

    // relations.json：存在则校验 0600 且 owner.address 与身份一致（磁盘上的 validate_owner 信号）
    let relations_path = agent_dir.join("relations.json");
    if relations_path.exists() {
        let perm_ok = path_mode(&relations_path) == Some(0o600);
        let owner_ok = relations::read(dot, &id.name)
            .map(|rf| rf.owner.address == id.address)
            .unwrap_or(false);
        let status = if perm_ok && owner_ok { "ok" } else { "warn" };
        let mut msg = String::new();
        if !perm_ok {
            msg.push_str("relations.json 权限不是 0600");
        }
        if !owner_ok {
            if !msg.is_empty() {
                msg.push('；');
            }
            msg.push_str("relations.owner.address 与当前身份不一致（可能曾写入错误 workspace）");
        }
        if msg.is_empty() {
            msg.push_str("relations.json 权限与 owner 均正常");
        }
        checks.push(check(
            "identity",
            "identity.relations_file",
            status,
            msg,
            if status == "ok" {
                None
            } else {
                Some("若 owner 不匹配，删除该 relations.json 后重新收发消息即可重建")
            },
            None,
            serde_json::json!({ "path": relations_path.display().to_string() }),
        ));
    }

    checks
}

#[cfg(unix)]
fn path_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn path_mode(_path: &Path) -> Option<u32> {
    Some(0o600)
}

// ---- message ----

fn message_checks(ctx: &DoctorContext, identity: &Option<ResolvedIdentity>) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let db_ok = ctx
        .storage
        .as_ref()
        .map(|s| s.conn().query_row("SELECT 1", [], |_| Ok(())).is_ok())
        .unwrap_or(false);
    checks.push(check(
        "message",
        "message.db",
        if db_ok { "ok" } else { "error" },
        if db_ok {
            "DB 可打开并执行 SELECT 1".to_string()
        } else {
            "DB 无法打开".to_string()
        },
        if db_ok {
            None
        } else {
            Some("检查 daemon 日志与数据库文件权限")
        },
        None,
        serde_json::Value::Null,
    ));

    let Some(id) = identity else {
        checks.push(check(
            "message",
            "message.mailbox",
            "skip",
            "身份未解析，跳过 inbox 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.pending",
            "skip",
            "身份未解析，跳过 pending 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.latest_event",
            "skip",
            "身份未解析，跳过 latest_event 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "message",
            "message.read_ready",
            "skip",
            "身份未解析，read 未就绪",
            Some("先执行 agtalk id join <name>"),
            Some("agtalk id join <name>"),
            serde_json::Value::Null,
        ));
        return checks;
    };

    let mailbox_ok = ctx
        .storage
        .as_ref()
        .and_then(|s| mailbox::get_by_address(s, &id.address).ok())
        .flatten()
        .is_some();
    checks.push(check(
        "message",
        "message.mailbox",
        if mailbox_ok { "ok" } else { "error" },
        if mailbox_ok {
            "当前 address 可查询 inbox".to_string()
        } else {
            "当前 address 在 DB 中无 mailbox".to_string()
        },
        if mailbox_ok {
            None
        } else {
            Some("重新 join 恢复 mailbox")
        },
        if mailbox_ok {
            None
        } else {
            Some("agtalk id join <name>")
        },
        serde_json::Value::Null,
    ));

    let pending = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::inbox(s, &id.address, false).ok())
        .map(|msgs| msgs.into_iter().filter(|m| m.status == "pending").count())
        .unwrap_or(0);
    checks.push(check(
        "message",
        "message.pending",
        "ok",
        format!("{} 条 pending 消息", pending),
        None,
        None,
        serde_json::json!({ "pending": pending }),
    ));

    let latest_event = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::max_event_id(s, &id.address).ok())
        .unwrap_or(0);
    checks.push(check(
        "message",
        "message.latest_event",
        "ok",
        format!("最新 event_id: {}", latest_event),
        None,
        None,
        serde_json::json!({ "latest_event_id": latest_event }),
    ));

    checks.push(check(
        "message",
        "message.read_ready",
        "ok",
        "agtalk msg read 可解析当前身份",
        None,
        None,
        serde_json::Value::Null,
    ));

    checks
}

// ---- wait / sse ----

fn wait_checks(ctx: &DoctorContext, identity: &Option<ResolvedIdentity>) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let http_url = format!("http://127.0.0.1:{}", ctx.config.http_port);
    let daemon_running = crate::server::daemon::read_status_file()
        .ok()
        .flatten()
        .is_some();
    let mut http_ok = false;

    if daemon_running {
        http_ok = http_get(&format!("{}/api/v1/daemon/status", http_url), &[]).is_ok();
        checks.push(check(
            "wait",
            "wait.http",
            if http_ok { "ok" } else { "error" },
            if http_ok {
                format!("daemon HTTP 可达: {}", http_url)
            } else {
                format!("daemon HTTP 不可达: {}", http_url)
            },
            if http_ok {
                None
            } else {
                Some("启动 daemon 后再使用 wait")
            },
            if http_ok {
                None
            } else {
                Some("agtalk daemon start")
            },
            serde_json::json!({ "url": http_url }),
        ));
    } else {
        checks.push(check(
            "wait",
            "wait.http",
            "skip",
            "daemon 未运行，跳过 HTTP 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    let Some(id) = identity else {
        checks.push(check(
            "wait",
            "wait.identity",
            "skip",
            "身份未解析，wait 无法认证",
            Some("先执行 agtalk id join <name>"),
            Some("agtalk id join <name>"),
            serde_json::Value::Null,
        ));
        checks.push(check(
            "wait",
            "wait.sse",
            "skip",
            "身份未解析，跳过 SSE 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "wait",
            "wait.replay",
            "skip",
            "身份未解析，跳过 replay 基线检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        return checks;
    };

    checks.push(check(
        "wait",
        "wait.identity",
        "ok",
        format!("身份已解析: {}", id.name),
        None,
        None,
        serde_json::Value::Null,
    ));

    if http_ok {
        let sse_url = format!("{}/api/v1/events", http_url);
        let headers = vec![
            ("X-AgTalk-Address", id.address.clone()),
            ("X-AgTalk-Pid", std::process::id().to_string()),
        ];
        let sse_ok = http_get_stream_head(&sse_url, &headers).is_ok();
        checks.push(check(
            "wait",
            "wait.sse",
            if sse_ok { "ok" } else { "error" },
            if sse_ok {
                "SSE 可建立连接".to_string()
            } else {
                "SSE 无法建立连接".to_string()
            },
            if sse_ok {
                None
            } else {
                Some("检查 daemon 日志与身份认证")
            },
            None,
            serde_json::json!({ "url": sse_url }),
        ));
    } else {
        checks.push(check(
            "wait",
            "wait.sse",
            "skip",
            "daemon HTTP 不可达，跳过 SSE 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
    }

    let latest_event = ctx
        .storage
        .as_ref()
        .and_then(|s| inbox::max_event_id(s, &id.address).ok())
        .unwrap_or(0);
    checks.push(check(
        "wait",
        "wait.replay",
        "ok",
        format!("replay 基线 event_id: {}", latest_event),
        None,
        None,
        serde_json::json!({ "latest_event_id": latest_event }),
    ));

    checks
}

// ---- notify ----

fn notify_checks(ctx: &DoctorContext, identity: &Option<ResolvedIdentity>) -> Vec<DiagnosisCheck> {
    let mut checks = Vec::new();

    let (detected_channel, detected_target) = notify::auto_detect();
    checks.push(check(
        "notify",
        "notify.environment",
        if detected_channel == "none" {
            "info"
        } else {
            "ok"
        },
        if detected_channel == "none" {
            "未检测到可用 notify plugin".to_string()
        } else {
            format!("检测到 {}", detected_channel)
        },
        if detected_channel == "none" {
            Some("安装 agtalk-notify-zellij / agtalk-notify-tmux 到 PATH 可获得终端通知")
        } else {
            None
        },
        None,
        serde_json::to_value(&detected_target).unwrap_or_default(),
    ));

    let Some(id) = identity else {
        checks.push(check(
            "notify",
            "notify.channel",
            "skip",
            "身份未解析，跳过 notify 通道检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        checks.push(check(
            "notify",
            "notify.target",
            "skip",
            "身份未解析，跳过 notify target 检查",
            None,
            None,
            serde_json::Value::Null,
        ));
        return checks;
    };

    let mb = ctx
        .storage
        .as_ref()
        .and_then(|s| mailbox::get_by_address(s, &id.address).ok())
        .flatten();
    let channel = mb
        .as_ref()
        .map(|m| m.notify_channel.clone())
        .unwrap_or_default();
    let target = mb
        .as_ref()
        .and_then(|m| serde_json::from_value::<NotifyTarget>(m.notify_target.clone()).ok());

    let (detected_channel, _detected_target) = notify::auto_detect();
    let channel_is_none = channel.is_empty() || channel == "none";
    if channel_is_none && detected_channel != "none" {
        checks.push(check(
            "notify",
            "notify.channel",
            "warn",
            format!(
                "notify 通道为 {}，但当前环境可用 {}",
                if channel.is_empty() { "none" } else { &channel },
                detected_channel
            ),
            Some("在对应终端环境内重新 join 以启用 notify"),
            Some(&format!("agtalk id join {} --notify auto", id.name)),
            serde_json::json!({ "channel": channel, "detected": detected_channel }),
        ));
    } else {
        checks.push(check(
            "notify",
            "notify.channel",
            "ok",
            format!(
                "notify 通道为 {}",
                if channel.is_empty() { "none" } else { &channel }
            ),
            None,
            None,
            serde_json::json!({ "channel": channel }),
        ));
    }

    let target_desc = match &target {
        Some(NotifyTarget::Plugin { name, endpoint }) => {
            format!("plugin {} endpoint: {}", name, endpoint)
        }
        _ => "none".to_string(),
    };
    checks.push(check(
        "notify",
        "notify.target",
        if target.is_some() && !matches!(target, Some(NotifyTarget::None)) {
            "ok"
        } else {
            "info"
        },
        &target_desc,
        None,
        None,
        serde_json::to_value(&target).unwrap_or_default(),
    ));

    // 插件通道：检查 discover + send --dry-run
    if let Some(NotifyTarget::Plugin { name, endpoint }) = target.as_ref() {
        let join_notify_cmd = format!("agtalk id join {} --notify plugin:{}", id.name, name);
        let set_plugin_path_cmd = format!(
            "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
            name
        );
        match crate::notify::plugin::PluginChannel::new(name) {
            Ok(plugin) => match plugin.resolve_binary() {
                Ok(path) => match crate::notify::plugin::PluginChannel::validate_binary(&path) {
                    Ok(()) => {
                        checks.push(check(
                            "notify",
                            "notify.plugin.binary",
                            "ok",
                            format!("插件 {} 二进制: {}", name, path.display()),
                            None,
                            None,
                            serde_json::json!({ "path": path.to_string_lossy() }),
                        ));

                        // discover
                        match plugin.discover() {
                            Ok(endpoint_result) => {
                                if endpoint_result.ready {
                                    checks.push(check(
                                        "notify",
                                        "notify.plugin.discover",
                                        "ok",
                                        format!(
                                            "插件 {} discover ready: {}",
                                            name, endpoint_result.message
                                        ),
                                        None,
                                        None,
                                        serde_json::to_value(&endpoint_result).unwrap_or_default(),
                                    ));

                                    // endpoint 一致性：session 中保存的 endpoint 和当前 discover 结果是否一致。
                                    if let Some(NotifyTarget::Plugin {
                                        endpoint: saved_endpoint,
                                        ..
                                    }) = target.as_ref()
                                    {
                                        if saved_endpoint != &endpoint_result.endpoint {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.endpoint_stale",
                                                "warn",
                                                format!(
                                                    "插件 {} endpoint 已过期（session 保存 {:?}，当前环境 {:?}）",
                                                    name, saved_endpoint, endpoint_result.endpoint
                                                ),
                                                Some("在当前终端环境内重新 join 以刷新 endpoint"),
                                                Some(&join_notify_cmd),
                                                serde_json::to_value(&endpoint_result)
                                                    .unwrap_or_default(),
                                            ));
                                        }
                                    }

                                    // dry-run
                                    let dummy = NotifyHint {
                                        from_name: "doctor".to_string(),
                                        binary_path: "agtalk".to_string(),
                                        agent_name: id.name.clone(),
                                        agent_address: id.address.clone(),
                                        message_id: "doctor-dry-run".to_string(),
                                        send_enter: true,
                                    };
                                    match plugin.send(endpoint, &dummy, true) {
                                        Ok(()) => {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.dry_run",
                                                "ok",
                                                format!("插件 {} send --dry-run 成功", name),
                                                None,
                                                None,
                                                serde_json::Value::Null,
                                            ));
                                        }
                                        Err(e) => {
                                            checks.push(check(
                                                "notify",
                                                "notify.plugin.dry_run",
                                                "error",
                                                format!("插件 {} send --dry-run 失败: {}", name, e),
                                                Some(
                                                    "检查插件是否能在当前环境访问对应终端/session",
                                                ),
                                                None,
                                                serde_json::Value::Null,
                                            ));
                                        }
                                    }
                                } else {
                                    checks.push(check(
                                        "notify",
                                        "notify.plugin.discover",
                                        "error",
                                        format!(
                                            "插件 {} discover 返回 not ready: {}",
                                            name, endpoint_result.message
                                        ),
                                        Some("在对应终端环境内重新 join，或安装可用插件"),
                                        Some(&join_notify_cmd),
                                        serde_json::to_value(&endpoint_result).unwrap_or_default(),
                                    ));
                                }
                            }
                            Err(e) => {
                                checks.push(check(
                                    "notify",
                                    "notify.plugin.discover",
                                    "error",
                                    format!("插件 {} discover 失败: {}", name, e),
                                    Some("检查插件是否可执行、配置路径是否正确"),
                                    Some(&set_plugin_path_cmd),
                                    serde_json::Value::Null,
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        checks.push(check(
                            "notify",
                            "notify.plugin.binary",
                            "error",
                            format!("插件 {} 不可执行: {}", name, e),
                            Some("检查文件权限或使用 chmod +x"),
                            Some(&set_plugin_path_cmd),
                            serde_json::Value::Null,
                        ));
                    }
                },
                Err(e) => {
                    let msg = e.to_string();
                    let (suggestion, command) = if msg.contains("找不到") {
                        (
                            format!(
                                "将可执行文件放入 {}（推荐）或在 PATH 中安装 agtalk-notify-{}",
                                crate::paths::plugins_dir()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|_| "<config_dir>/plugins".to_string()),
                                name
                            ),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    } else if msg.contains("'..'") || msg.contains("逃逸") {
                        (
                            "插件相对路径禁止包含 '..'".to_string(),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    } else {
                        (
                            format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            ),
                            Some(format!(
                                "agtalk config set notify.plugins.{}.path <name-or-abs-path>",
                                name
                            )),
                        )
                    };
                    checks.push(check(
                        "notify",
                        "notify.plugin.binary",
                        "error",
                        format!("插件 {} 不可用: {}", name, msg),
                        Some(&suggestion),
                        command.as_deref(),
                        serde_json::Value::Null,
                    ));
                }
            },
            Err(e) => {
                checks.push(check(
                    "notify",
                    "notify.plugin.binary",
                    "error",
                    format!("非法插件名 {}: {}", name, e),
                    None,
                    None,
                    serde_json::Value::Null,
                ));
            }
        }
    }

    checks
}

// ---- http helpers ----

fn http_get(url: &str, headers: &[(&str, String)]) -> Result<serde_json::Value, String> {
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

fn http_get_stream_head(url: &str, headers: &[(&str, String)]) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::session_file::SessionFile;
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
}
