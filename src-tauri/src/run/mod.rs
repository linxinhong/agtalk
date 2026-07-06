//! YAML 编排执行：把一系列 agtalk 动作写成文件运行。

use crate::config::AgConfig;
use crate::proto::{AskOptions, InboxFilter, RunResult, RunStepError, RunStepResult, ServerMsg};
use crate::server::handlers::{config, id, mem, msg};
use crate::server::state::AppState;
use crate::storage::Storage;
use axum::http::HeaderMap;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// 执行编排所需的认证上下文。
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub address: String,
    pub pid: u32,
    pub start_time: u64,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML 解析错误: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("JSON 解析错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("动作 {action} 缺少字段 {field}")]
    MissingField { action: String, field: String },
    #[error("配置错误: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("未指定编排文件")]
    MissingFile,
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Deserialize)]
struct RunSpec {
    #[serde(rename = "version")]
    _version: u32,
    steps: Vec<RunStep>,
}

#[derive(Debug, Deserialize)]
struct RunStep {
    action: String,
    #[serde(flatten)]
    fields: serde_yaml::Mapping,
}

const ALLOWED_ACTIONS: &[&str] = &[
    "id.show",
    "id.lookup",
    "msg.send",
    "msg.reply",
    "msg.done",
    "msg.ask",
    "msg.wait",
    "msg.read",
    "msg.inbox",
    "mem.plan.show",
    "mem.plan.update",
    "mem.plan.status",
    "mem.pack",
    "config.get",
];

/// 执行 YAML 编排文件。
pub fn run_file(
    storage: &Storage,
    dot_agtalk: &Path,
    ctx: Option<&AuthContext>,
    file: Option<PathBuf>,
) -> Result<RunResult, RunError> {
    let path = file.ok_or(RunError::MissingFile)?;
    let content = std::fs::read_to_string(&path)?;
    let spec: RunSpec = serde_yaml::from_str(&content)?;

    let config = AgConfig::load()?;
    let state = AppState::new(storage.clone(), config, dot_agtalk.to_path_buf());

    let mut results = Vec::new();
    let mut stopped_at = None;
    for (index, step) in spec.steps.into_iter().enumerate() {
        let index = index + 1;
        let result = execute_step(&state, ctx, &step.action, &step.fields);
        let status = if matches!(result, ServerMsg::Error { .. }) {
            "error".to_string()
        } else {
            "ok".to_string()
        };
        let (error, output) = match result {
            ServerMsg::Error { code, message } => (
                Some(RunStepError { code, message }),
                serde_json::Value::Null,
            ),
            other => (
                None,
                serde_json::to_value(&other).unwrap_or_else(|_| serde_json::json!({})),
            ),
        };
        results.push(RunStepResult {
            index,
            action: step.action,
            status: status.clone(),
            error,
            output,
        });
        if status == "error" {
            stopped_at = Some(index);
            break;
        }
    }

    let status = if stopped_at.is_some() {
        "error".to_string()
    } else {
        "ok".to_string()
    };
    Ok(RunResult {
        status,
        file: Some(path.to_string_lossy().into_owned()),
        steps: results,
        stopped_at,
    })
}

fn execute_step(
    state: &AppState,
    ctx: Option<&AuthContext>,
    action: &str,
    fields: &serde_yaml::Mapping,
) -> ServerMsg {
    if !ALLOWED_ACTIONS.contains(&action) {
        return ServerMsg::Error {
            code: "action_not_allowed".into(),
            message: format!("{} 不在允许的动作白名单中", action),
        };
    }

    let headers = ctx.map(auth_headers);

    macro_rules! field {
        ($key:expr) => {
            match require_field::<String>(fields, action, $key) {
                Ok(v) => v,
                Err(e) => return run_error(e),
            }
        };
        ($key:expr, default) => {
            match field_opt::<String>(fields, $key) {
                Ok(v) if !v.is_empty() => v,
                Ok(_) => Default::default(),
                Err(e) => return run_error(e),
            }
        };
        ($key:expr, bool default = $def:expr) => {
            match field_opt::<bool>(fields, $key) {
                Ok(v) => v,
                Err(e) => return run_error(e),
            }
        };
        ($key:expr, option) => {
            match field_opt::<Option<String>>(fields, $key) {
                Ok(v) => v,
                Err(e) => return run_error(e),
            }
        };
        ($key:expr, vec) => {
            match field_opt::<Vec<String>>(fields, $key) {
                Ok(v) => v,
                Err(e) => return run_error(e),
            }
        };
        ($key:expr, opt_usize) => {
            match field_opt::<Option<usize>>(fields, $key) {
                Ok(v) => v,
                Err(e) => return run_error(e),
            }
        };
    }

    match action {
        "id.show" => match headers {
            Some(ref h) => id::handle_show(state, h),
            None => auth_required_error(),
        },
        "id.lookup" => {
            let name: Option<String> = field!("name", option);
            id::handle_lookup(state, name)
        }
        "msg.send" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let to: String = field!("to");
            let body: String = field!("body");
            let subject: Option<String> = field!("subject", option);
            let files: Vec<String> = field!("files", vec);
            let notify: bool = field!("notify", bool default = false);
            let more: bool = field!("more", bool default = false);
            msg::handle_send(
                state,
                &headers,
                to,
                body,
                subject,
                files,
                Some(notify),
                more,
            )
        }
        "msg.reply" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let message_id: String = field!("message_id");
            let body: String = field!("body");
            let files: Vec<String> = field!("files", vec);
            let notify: bool = field!("notify", bool default = false);
            msg::handle_reply(state, &headers, message_id, body, files, Some(notify))
        }
        "msg.done" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let message_id: Option<String> = field!("message_id", option);
            let body: Option<String> = field!("body", option);
            let files: Vec<String> = field!("files", vec);
            msg::handle_done(state, &headers, message_id, body, files)
        }
        "msg.ask" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let message: String = field!("message");
            let options: AskOptions = match field_opt::<AskOptions>(fields, "options") {
                Ok(v) => v,
                Err(e) => return run_error(e),
            };
            let wait: bool = field!("wait", bool default = false);
            let timeout: Option<u64> = match field_opt::<Option<u64>>(fields, "timeout") {
                Ok(v) => v,
                Err(e) => return run_error(e),
            };
            msg::handle_ask(state, &headers, message, Vec::new(), options, wait, timeout)
        }
        "msg.wait" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let message_id: Option<String> = field!("message_id", option);
            let timeout: Option<u64> = match field_opt::<Option<u64>>(fields, "timeout") {
                Ok(v) => v,
                Err(e) => return run_error(e),
            };
            let since: Option<i64> = match field_opt::<Option<i64>>(fields, "since") {
                Ok(v) => v,
                Err(e) => return run_error(e),
            };
            msg::handle_wait(state, &headers, message_id, timeout, since)
        }
        "msg.read" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let message_id: Option<String> = field!("message_id", option);
            msg::handle_read(state, &headers, message_id)
        }
        "msg.inbox" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let all: bool = field!("all", bool default = false);
            msg::handle_inbox(
                state,
                &headers,
                InboxFilter {
                    all,
                    ..Default::default()
                },
            )
        }
        "mem.plan.show" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let target: Option<String> = field!("target", option);
            mem::handle_plan_show(state, &headers, target)
        }
        "mem.plan.update" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let plan: Option<String> = field!("plan", option);
            let context: Option<String> = field!("context", option);
            let status: Option<String> = field!("status", option);
            let summary: Option<String> = field!("summary", option);
            mem::handle_plan_update(state, &headers, plan, context, status, summary)
        }
        "mem.plan.status" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let target: Option<String> = field!("target", option);
            mem::handle_plan_status(state, &headers, target)
        }
        "mem.pack" => {
            let headers = match headers {
                Some(h) => h,
                None => return auth_required_error(),
            };
            let topic: Option<String> = field!("topic", option);
            let limit: Option<usize> = field!("limit", opt_usize);
            mem::handle_pack(state, &headers, topic, limit)
        }
        "config.get" => {
            let key: String = field!("key");
            config::handle_get(state, key)
        }
        _ => ServerMsg::Error {
            code: "action_not_allowed".into(),
            message: format!("{} 不在允许的动作白名单中", action),
        },
    }
}

fn auth_headers(ctx: &AuthContext) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-AgTalk-Address",
        ctx.address.parse().expect("address header 非法"),
    );
    headers.insert(
        "X-AgTalk-Pid",
        ctx.pid.to_string().parse().expect("pid header 非法"),
    );
    headers.insert(
        "X-AgTalk-Start-Time",
        ctx.start_time
            .to_string()
            .parse()
            .expect("start_time header 非法"),
    );
    headers
}

fn auth_required_error() -> ServerMsg {
    ServerMsg::Error {
        code: "auth_required".into(),
        message: "该步骤需要认证上下文".into(),
    }
}

fn run_error(e: RunError) -> ServerMsg {
    ServerMsg::Error {
        code: "run_step_failed".into(),
        message: e.to_string(),
    }
}

fn require_field<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    action: &str,
    key: &str,
) -> Result<T, RunError> {
    get_field(fields, action, key)
}

fn get_field<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    action: &str,
    key: &str,
) -> Result<T, RunError> {
    match fields.get(serde_yaml::Value::String(key.into())) {
        Some(v) => {
            let json = serde_json::to_value(v)?;
            serde_json::from_value(json).map_err(RunError::Json)
        }
        None => Err(RunError::MissingField {
            action: action.into(),
            field: key.into(),
        }),
    }
}

fn get_field_opt<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    key: &str,
) -> Result<Option<T>, RunError> {
    match fields.get(serde_yaml::Value::String(key.into())) {
        Some(v) => {
            let json = serde_json::to_value(v)?;
            Ok(Some(serde_json::from_value(json).map_err(RunError::Json)?))
        }
        None => Ok(None),
    }
}

fn field_opt<T: DeserializeOwned + Default>(
    fields: &serde_yaml::Mapping,
    key: &str,
) -> Result<T, RunError> {
    get_field_opt(fields, key).map(|opt| opt.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::agents_map;
    use crate::identity::mailbox;
    use crate::identity::session_file;
    use crate::identity::session_file::{NotifyTarget, SessionFile};
    use std::ffi::OsString;
    use sysinfo::{Pid, System};
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

    fn current_start_time() -> u64 {
        let pid = std::process::id();
        let mut sys = System::new_all();
        sys.refresh_processes();
        sys.process(Pid::from(pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1)
    }

    fn setup_sender(dot: &std::path::Path, storage: &Storage) -> (String, AuthContext) {
        let address = mailbox::create(storage, "runner", "runner", "default").unwrap();
        let session = SessionFile {
            address: address.clone(),
            name: "runner".to_string(),
            workspace: "default".to_string(),
            intro: "runner".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            command: "agtalk".to_string(),
            notify_channel: "none".to_string(),
            notify_target: NotifyTarget::None,
        };
        session_file::write(dot, "runner", &session).unwrap();
        let start_time = current_start_time();
        agents_map::register_pid(dot, std::process::id(), "runner", start_time).unwrap();
        (
            address.clone(),
            AuthContext {
                address,
                pid: std::process::id(),
                start_time,
            },
        )
    }

    #[tokio::test]
    async fn run_yaml_sends_message() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let (_sender_addr, ctx) = setup_sender(&dot, &storage);
        let recipient = mailbox::create(&storage, "receiver", "receiver", "default").unwrap();

        let yaml = format!(
            r#"
version: 1
steps:
  - action: msg.send
    to: "{}"
    body: "hello from yaml"
"#,
            recipient
        );
        let run_path = tmp.path().join("run.yaml");
        std::fs::write(&run_path, yaml).unwrap();

        let result = run_file(&storage, &dot, Some(&ctx), Some(run_path)).unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].status, "ok");

        let conn = storage.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE to_address = ?1 AND body = ?2",
                [&recipient, "hello from yaml"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn run_yaml_stops_on_error() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let (_sender_addr, ctx) = setup_sender(&dot, &storage);

        let yaml = r#"
version: 1
steps:
  - action: msg.send
    to: "not-a-uuid"
    body: "boom"
  - action: id.lookup
"#;
        let run_path = tmp.path().join("run.yaml");
        std::fs::write(run_path, yaml).unwrap();

        let result = run_file(
            &storage,
            &dot,
            Some(&ctx),
            Some(tmp.path().join("run.yaml")),
        )
        .unwrap();
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].status, "error");
    }
}
