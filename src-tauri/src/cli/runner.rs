//! CLI-local YAML runner for `agtalk run`。
//!
//! runner 在 CLI 本地解析 YAML、校验白名单并逐步执行：
//! - 需要 daemon 的动作走现有 HTTP client；
//! - `msg.wait` 复用 CLI 本地 SSE 实现；
//! - `tool.doctor` 复用本地 doctor 诊断。

use crate::cli::client::msg::wait_result;
use crate::cli::client::{get, patch, post};
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use crate::config::AgConfig;
use crate::proto::{AskOptions, RunStepError, RunStepResult, ServerMsg};
use crate::storage::Storage;
use crate::tool::DoctorContext;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::PathBuf;

const ALLOWED_ACTIONS: &[&str] = &[
    "id.show",
    "id.lookup",
    "msg.send",
    "msg.reply",
    "msg.done",
    "msg.ask",
    "msg.read",
    "msg.inbox",
    "msg.wait",
    "mem.plan.show",
    "mem.plan.update",
    "mem.plan.status",
    "mem.pack",
    "config.get",
    "tool.doctor",
];

#[derive(Debug, serde::Deserialize)]
struct RunSpec {
    version: u32,
    steps: Vec<RunStep>,
}

#[derive(Debug, serde::Deserialize)]
struct RunStep {
    action: String,
    #[serde(flatten)]
    fields: serde_yaml::Mapping,
}

#[derive(Debug, Clone)]
struct StepError {
    code: String,
    message: String,
}

impl StepError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<CliError> for StepError {
    fn from(e: CliError) -> Self {
        Self::new(e.code, e.message)
    }
}

/// 执行 YAML 编排文件。
pub fn run(ctx: Context, file: Option<PathBuf>, json: bool) -> Result<(), CliError> {
    let path = resolve_run_file(&ctx, file)?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| CliError::new("run_file_missing", format!("无法读取运行文件: {}", e)))?;
    let spec: RunSpec = serde_yaml::from_str(&content)
        .map_err(|e| CliError::new("yaml_parse_error", format!("YAML 解析失败: {}", e)))?;

    if spec.version != 1 {
        return Err(CliError::new(
            "unsupported_version",
            format!("不支持的 run 版本: {}", spec.version),
        ));
    }

    let mut results = Vec::new();
    let mut stopped_at = None;
    for (idx, step) in spec.steps.into_iter().enumerate() {
        let index = idx + 1;
        let result = execute_step(&ctx, &step).unwrap_or_else(|e| ServerMsg::Error {
            code: e.code,
            message: e.message,
        });
        let is_error = matches!(result, ServerMsg::Error { .. });
        results.push(build_step_result(index, step.action, result));
        if is_error {
            stopped_at = Some(index);
            break;
        }
    }

    let status = if stopped_at.is_some() {
        "error".to_string()
    } else {
        "ok".to_string()
    };

    let result = ServerMsg::RunResult {
        status,
        file: Some(path.to_string_lossy().into_owned()),
        steps: results,
        stopped_at,
    };

    print_server_msg(json, &result);
    Ok(())
}

fn resolve_run_file(ctx: &Context, file: Option<PathBuf>) -> Result<PathBuf, CliError> {
    match file {
        Some(p) => Ok(p),
        None => {
            let path = ctx
                .dot_agtalk
                .join("runs")
                .join(format!("{}.yaml", ctx.name));
            if !path.exists() {
                return Err(CliError::new(
                    "run_file_missing",
                    format!("默认运行文件不存在: {}", path.display()),
                ));
            }
            Ok(path)
        }
    }
}

fn execute_step(ctx: &Context, step: &RunStep) -> Result<ServerMsg, StepError> {
    if !ALLOWED_ACTIONS.contains(&step.action.as_str()) {
        return Err(StepError::new(
            "action_not_allowed",
            format!("{} 不在允许的动作白名单中", step.action),
        ));
    }

    macro_rules! field {
        ($key:expr) => {
            require_field::<String>(&step.fields, &step.action, $key)?
        };
        ($key:expr, option) => {
            field_opt::<Option<String>>(&step.fields, $key)?
        };
        ($key:expr, bool) => {
            field_opt::<bool>(&step.fields, $key)?
        };
        ($key:expr, vec) => {
            field_opt::<Vec<String>>(&step.fields, $key)?
        };
        ($key:expr, opt_usize) => {
            field_opt::<Option<usize>>(&step.fields, $key)?
        };
        ($key:expr, opt_u64) => {
            field_opt::<Option<u64>>(&step.fields, $key)?
        };
        ($key:expr, opt_i64) => {
            field_opt::<Option<i64>>(&step.fields, $key)?
        };
        ($key:expr, ask) => {
            field_opt::<AskOptions>(&step.fields, $key)?
        };
    }

    match step.action.as_str() {
        "id.show" => http_get(ctx, "/api/v1/id/me"),
        "id.lookup" => {
            let name: Option<String> = field!("name", option);
            let endpoint = match name {
                Some(n) => format!("/api/v1/id/lookup?name={}", n),
                None => "/api/v1/id/lookup".into(),
            };
            http_get(ctx, &endpoint)
        }
        "msg.send" => {
            let body = serde_json::json!({
                "to": field!("to"),
                "body": field!("body"),
                "subject": field!("subject", option),
                "files": field!("files", vec),
                "notify": field!("notify", bool),
                "more": field!("more", bool),
            });
            http_post(ctx, "/api/v1/msg/send", body)
        }
        "msg.reply" => {
            let body = serde_json::json!({
                "message_id": field!("message_id"),
                "body": field!("body"),
                "files": field!("files", vec),
                "notify": field!("notify", bool),
            });
            http_post(ctx, "/api/v1/msg/reply", body)
        }
        "msg.done" => {
            let body = serde_json::json!({
                "message_id": field!("message_id", option),
                "body": field!("body", option),
                "files": field!("files", vec),
            });
            http_post(ctx, "/api/v1/msg/done", body)
        }
        "msg.ask" => {
            let body = serde_json::json!({
                "message": field!("message"),
                "options": field!("options", ask),
                "wait": field!("wait", bool),
                "timeout": field!("timeout", opt_u64),
            });
            http_post(ctx, "/api/v1/msg/ask", body)
        }
        "msg.read" => {
            let message_id: Option<String> = field!("message_id", option);
            http_post(
                ctx,
                "/api/v1/msg/read",
                serde_json::json!({ "message_id": message_id }),
            )
        }
        "msg.inbox" => {
            let all: bool = field!("all", bool);
            let limit: Option<usize> = field!("limit", opt_usize);
            let mut endpoint = "/api/v1/msg/inbox".to_string();
            let mut first = true;
            if all {
                endpoint.push_str("?all=true");
                first = false;
            }
            if let Some(l) = limit {
                endpoint.push_str(&format!("{}limit={}", if first { "?" } else { "&" }, l));
            }
            http_get(ctx, &endpoint)
        }
        "msg.wait" => {
            let message_id: Option<String> = field!("message_id", option);
            let timeout: Option<u64> = field!("timeout", opt_u64);
            let since: Option<i64> = field!("since", opt_i64);
            wait_result(ctx.clone(), message_id, timeout, since).map_err(Into::into)
        }
        "mem.plan.show" => {
            let target: Option<String> = field!("target", option);
            let endpoint = match target {
                Some(t) => format!("/api/v1/mem/plan?target={}", t),
                None => "/api/v1/mem/plan".into(),
            };
            http_get(ctx, &endpoint)
        }
        "mem.plan.update" => {
            let body = serde_json::json!({
                "plan": field!("plan", option),
                "context": field!("context", option),
                "status": field!("status", option),
                "summary": field!("summary", option),
            });
            http_patch(ctx, "/api/v1/mem/plan", body)
        }
        "mem.plan.status" => {
            let target: Option<String> = field!("target", option);
            let endpoint = match target {
                Some(t) => format!("/api/v1/mem/plan/status?target={}", t),
                None => "/api/v1/mem/plan/status".into(),
            };
            http_get(ctx, &endpoint)
        }
        "mem.pack" => {
            let topic: Option<String> = field!("topic", option);
            let limit: Option<usize> = field!("limit", opt_usize);
            let mut endpoint = "/api/v1/mem/pack".to_string();
            let mut first = true;
            if let Some(t) = topic {
                endpoint.push_str(&format!("{}topic={}", if first { "?" } else { "&" }, t));
                first = false;
            }
            if let Some(l) = limit {
                endpoint.push_str(&format!("{}limit={}", if first { "?" } else { "&" }, l));
            }
            http_get(ctx, &endpoint)
        }
        "config.get" => {
            let key: String = field!("key");
            http_get(ctx, &format!("/api/v1/config/{}", key))
        }
        "tool.doctor" => Ok(run_tool_doctor(ctx)),
        _ => Err(StepError::new(
            "action_not_allowed",
            format!("{} 不在允许的动作白名单中", step.action),
        )),
    }
}

fn http_get(ctx: &Context, endpoint: &str) -> Result<ServerMsg, StepError> {
    get(ctx, endpoint).map_err(Into::into)
}

fn http_post(ctx: &Context, endpoint: &str, body: impl Serialize) -> Result<ServerMsg, StepError> {
    post(ctx, endpoint, body).map_err(Into::into)
}

fn http_patch(ctx: &Context, endpoint: &str, body: impl Serialize) -> Result<ServerMsg, StepError> {
    patch(ctx, endpoint, body).map_err(Into::into)
}

fn run_tool_doctor(ctx: &Context) -> ServerMsg {
    let config = match AgConfig::load() {
        Ok(c) => c,
        Err(e) => {
            return ServerMsg::Error {
                code: "config_load".into(),
                message: e.to_string(),
            }
        }
    };
    let storage = Storage::open().ok();
    let doc_ctx = DoctorContext::new(
        ctx.dot_agtalk.clone(),
        config,
        storage,
        Some(ctx.name.clone()),
    );
    crate::tool::doctor::run(doc_ctx)
}

fn build_step_result(index: usize, action: String, result: ServerMsg) -> RunStepResult {
    match result {
        ServerMsg::Error { code, message } => RunStepResult {
            index,
            action,
            status: "error".to_string(),
            error: Some(RunStepError { code, message }),
            output: serde_json::Value::Null,
        },
        other => RunStepResult {
            index,
            action,
            status: "ok".to_string(),
            error: None,
            output: serde_json::to_value(&other).unwrap_or_default(),
        },
    }
}

fn require_field<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    action: &str,
    key: &str,
) -> Result<T, StepError> {
    get_field(fields, action, key)
}

fn get_field<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    action: &str,
    key: &str,
) -> Result<T, StepError> {
    match fields.get(serde_yaml::Value::String(key.into())) {
        Some(v) => {
            let json = serde_json::to_value(v)
                .map_err(|e| StepError::new("field_conversion", e.to_string()))?;
            serde_json::from_value(json)
                .map_err(|e| StepError::new("field_conversion", format!("{}: {}", key, e)))
        }
        None => Err(StepError::new(
            "missing_field",
            format!("{} 缺少字段 {}", action, key),
        )),
    }
}

fn field_opt<T: DeserializeOwned + Default>(
    fields: &serde_yaml::Mapping,
    key: &str,
) -> Result<T, StepError> {
    get_field_opt(fields, key).map(|opt| opt.unwrap_or_default())
}

fn get_field_opt<T: DeserializeOwned>(
    fields: &serde_yaml::Mapping,
    key: &str,
) -> Result<Option<T>, StepError> {
    match fields.get(serde_yaml::Value::String(key.into())) {
        Some(v) => {
            let json = serde_json::to_value(v)
                .map_err(|e| StepError::new("field_conversion", e.to_string()))?;
            Ok(Some(serde_json::from_value(json).map_err(|e| {
                StepError::new("field_conversion", format!("{}: {}", key, e))
            })?))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_run_spec() {
        let yaml = r#"
version: 1
steps:
  - action: id.show
  - action: msg.send
    to: "550e8400-e29b-41d4-a716-446655440000"
    body: "hello"
"#;
        let spec: RunSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.version, 1);
        assert_eq!(spec.steps.len(), 2);
        assert_eq!(spec.steps[0].action, "id.show");
        assert_eq!(spec.steps[1].action, "msg.send");
    }

    #[test]
    fn unknown_action_not_allowed() {
        let step = RunStep {
            action: "shell".into(),
            fields: serde_yaml::Mapping::new(),
        };
        let ctx = Context {
            dot_agtalk: PathBuf::from("."),
            address: "addr".into(),
            name: "test".into(),
            pid: 1,
            start_time: 1,
            base_url: "http://127.0.0.1:19527".into(),
        };
        let result = execute_step(&ctx, &step).unwrap_err();
        assert_eq!(result.code, "action_not_allowed");
    }

    #[test]
    fn missing_field_returns_error() {
        let step = RunStep {
            action: "msg.send".into(),
            fields: serde_yaml::Mapping::new(),
        };
        let ctx = Context {
            dot_agtalk: PathBuf::from("."),
            address: "addr".into(),
            name: "test".into(),
            pid: 1,
            start_time: 1,
            base_url: "http://127.0.0.1:19527".into(),
        };
        let result = execute_step(&ctx, &step).unwrap_err();
        assert_eq!(result.code, "missing_field");
    }

    #[test]
    fn build_ok_step_result() {
        let result = build_step_result(1, "id.show".into(), ServerMsg::Pong);
        assert_eq!(result.status, "ok");
        assert!(result.error.is_none());
        assert_eq!(result.index, 1);
    }

    #[test]
    fn build_error_step_result() {
        let result = build_step_result(
            3,
            "mem.plan.update".into(),
            ServerMsg::Error {
                code: "missing_field".into(),
                message: "missing summary".into(),
            },
        );
        assert_eq!(result.status, "error");
        assert_eq!(result.error.as_ref().unwrap().code, "missing_field");
    }
}
