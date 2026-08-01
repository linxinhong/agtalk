//! CLI graph 子命令（docs/design_graph.md §7）。
//!
//! submit（CLI 侧自动探测当前目录 git）/ list / status / logs / cancel / node report。
//! node report 是 participant（执行 agent）的上报入口，对应 graph-participant-protocol.md §4。

use crate::cli::client;
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use serde_json::json;
use std::path::PathBuf;

use super::GraphCmd;

pub(crate) fn dispatch(ctx: Context, cmd: GraphCmd, json: bool) -> Result<(), CliError> {
    match cmd {
        GraphCmd::Submit { spec } => submit(&ctx, &spec, json),
        GraphCmd::List { status } => list(&ctx, status, json),
        GraphCmd::Status { run_id } => status(&ctx, &run_id, json),
        GraphCmd::Logs { run_id, since } => logs(&ctx, &run_id, since, json),
        GraphCmd::Cancel { run_id } => cancel(&ctx, &run_id, json),
        GraphCmd::Node { cmd } => match cmd {
            super::GraphNodeCmd::Heartbeat {
                run_id,
                node_key,
                attempt,
            } => heartbeat(&ctx, &run_id, &node_key, attempt, json),
            super::GraphNodeCmd::Result {
                run_id,
                node_key,
                attempt,
                file,
            } => result(&ctx, &run_id, &node_key, attempt, &file, json),
            super::GraphNodeCmd::Blocker {
                run_id,
                node_key,
                attempt,
                blocker,
            } => report_blocker(&ctx, &run_id, &node_key, attempt, &blocker, json),
        },
        GraphCmd::Gui { run_id } => {
            let _ = run_id; // M4：启动图工程管理界面（?view=graph），run 选择在 GUI 内进行
            crate::run_graph_gui();
            Ok(())
        }
    }
}

/// 提交 spec。repository/base_revision 由 CLI 侧探测当前目录 git（design_graph.md §7），
/// 存入 spec 顶层字段后随请求发送（daemon 侧优先 spec 内值）。
pub fn submit(ctx: &Context, spec_file: &PathBuf, json: bool) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(spec_file)
        .map_err(|e| CliError::from(format!("读取 spec 失败 {}: {}", spec_file.display(), e)))?;
    // 探测当前目录 git（失败不阻塞，daemon 用空值）
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw.clone()));
    let detect = detect_git();
    if let Some(v) = detect {
        for (k, val) in [("repository", v.0), ("base_revision", v.1)] {
            if value.get(k).map(|x| x.is_null()).unwrap_or(true) {
                value[k] = json!(val);
            }
        }
    }
    let spec_text = if let serde_json::Value::String(s) = &value {
        s.clone()
    } else {
        value.to_string()
    };
    let msg = client::post(ctx, "/api/v1/graph/submit", json!({ "spec": spec_text }))?;
    print_server_msg(json, &msg);
    Ok(())
}

pub fn list(ctx: &Context, status: Option<String>, json: bool) -> Result<(), CliError> {
    let path = match status {
        Some(s) => format!("/api/v1/graph/runs?status={}", s),
        None => "/api/v1/graph/runs".to_string(),
    };
    let msg = client::get(ctx, &path)?;
    print_server_msg(json, &msg);
    Ok(())
}

pub fn status(ctx: &Context, run_id: &str, json: bool) -> Result<(), CliError> {
    let msg = client::get(ctx, &format!("/api/v1/graph/runs/{run_id}"))?;
    print_server_msg(json, &msg);
    Ok(())
}

pub fn logs(ctx: &Context, run_id: &str, since: Option<i64>, json: bool) -> Result<(), CliError> {
    let path = format!(
        "/api/v1/graph/runs/{run_id}/events{}",
        since.map(|s| format!("?since={s}")).unwrap_or_default()
    );
    let msg = client::get(ctx, &path)?;
    print_server_msg(json, &msg);
    Ok(())
}

pub fn cancel(ctx: &Context, run_id: &str, json: bool) -> Result<(), CliError> {
    let msg = client::post(
        ctx,
        &format!("/api/v1/graph/runs/{run_id}/control"),
        json!({ "action": "cancel" }),
    )?;
    print_server_msg(json, &msg);
    Ok(())
}

fn heartbeat(
    ctx: &Context,
    run_id: &str,
    node_key: &str,
    attempt: u32,
    json: bool,
) -> Result<(), CliError> {
    let msg = client::post(
        ctx,
        "/api/v1/graph/node/heartbeat",
        json!({ "run_id": run_id, "node_key": node_key, "attempt": attempt }),
    )?;
    print_server_msg(json, &msg);
    Ok(())
}

/// result：读取 result.json（graph-participant-protocol.md §6），
/// 合并 run_id/node_key/attempt 后提交候选结果。
fn result(
    ctx: &Context,
    run_id: &str,
    node_key: &str,
    attempt: u32,
    file: &PathBuf,
    json: bool,
) -> Result<(), CliError> {
    let raw = std::fs::read_to_string(file)
        .map_err(|e| CliError::from(format!("读取 result 失败 {}: {}", file.display(), e)))?;
    let mut body: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::from(format!("result 不是合法 JSON: {e}")))?;
    body["run_id"] = json!(run_id);
    body["node_key"] = json!(node_key);
    body["attempt"] = json!(attempt);
    let msg = client::post(ctx, "/api/v1/graph/node/result", body)?;
    print_server_msg(json, &msg);
    Ok(())
}

fn report_blocker(
    ctx: &Context,
    run_id: &str,
    node_key: &str,
    attempt: u32,
    blocker: &str,
    json: bool,
) -> Result<(), CliError> {
    let msg = client::post(
        ctx,
        "/api/v1/graph/node/result",
        json!({
            "run_id": run_id,
            "node_key": node_key,
            "attempt": attempt,
            "result": "",
            "blockers": [blocker]
        }),
    )?;
    print_server_msg(json, &msg);
    Ok(())
}

/// 探测当前目录 git 的 (remote, branch/HEAD)，失败返回 None。
fn detect_git() -> Option<(String, String)> {
    let run = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let remote = run(&["remote", "get-url", "origin"]);
    let branch =
        run(&["branch", "--show-current"]).or_else(|| run(&["rev-parse", "--short", "HEAD"]));
    match (remote, branch) {
        (Some(r), Some(b)) => Some((r, b)),
        _ => None,
    }
}
