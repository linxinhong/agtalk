//! CLI graph 子命令（docs/design_graph.md §7）。
//!
//! submit（CLI 侧自动探测当前目录 git）/ list / status / logs / cancel / node report。
//! node report 是 participant（执行 agent）的上报入口，对应 graph-participant-protocol.md §4。

use crate::cli::client;
use crate::cli::context::Context;
use crate::cli::output::{print_server_msg, CliError};
use serde_json::json;
use std::path::{Path, PathBuf};

use super::GraphCmd;

pub(crate) fn dispatch(ctx: Context, cmd: GraphCmd, json: bool) -> Result<(), CliError> {
    match cmd {
        GraphCmd::Submit { spec } => submit(&ctx, &spec, json),
        GraphCmd::List { status } => list(&ctx, status, json),
        GraphCmd::Status { run_id } => status(&ctx, &run_id, json),
        GraphCmd::Logs { run_id, since } => logs(&ctx, &run_id, since, json),
        GraphCmd::Cancel { run_id } => cancel(&ctx, &run_id, json),
        GraphCmd::Patch { run_id, spec } => patch(&ctx, &run_id, &spec, json),
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
        GraphCmd::Analyze { spec } => analyze(&ctx, &spec, json),
        GraphCmd::Gui { run_id } => {
            let _ = run_id; // M4：启动图工程管理界面（?view=graph），run 选择在 GUI 内进行
            crate::run_graph_gui();
            Ok(())
        }
    }
}

/// 解析 spec 路径：显式路径（存在/绝对/含分隔符/带扩展名）直接用；
/// 否则视为名字，从 `<cwd>/.agtalk/graph/<name>.yaml` 读取（自动补 .yaml 后缀）。
fn resolve_spec(ctx: &Context, spec: &Path) -> Result<PathBuf, CliError> {
    if spec.exists()
        || spec.is_absolute()
        || spec.components().count() > 1
        || spec.extension().is_some()
    {
        return Ok(spec.to_path_buf());
    }
    let mut name = spec.to_string_lossy().into_owned();
    name.push_str(".yaml");
    let dir = ctx.dot_agtalk.join("graph");
    std::fs::create_dir_all(&dir)
        .map_err(|e| CliError::from(format!("创建 .agtalk/graph 失败: {e}")))?;
    Ok(dir.join(name))
}

/// 提交 spec。repository/base_revision 由 CLI 侧探测当前目录 git（design_graph.md §7），
/// 存入 spec 顶层字段后随请求发送（daemon 侧优先 spec 内值）。
pub fn submit(ctx: &Context, spec_file: &Path, json: bool) -> Result<(), CliError> {
    // spec 解析：显式路径直接用；否则约定 <cwd>/.agtalk/graph/<name>.yaml
    let resolved = resolve_spec(ctx, spec_file)?;
    let raw = std::fs::read_to_string(&resolved)
        .map_err(|e| CliError::from(format!("读取 spec 失败 {}: {}", resolved.display(), e)))?;
    // 探测当前目录 git（失败不阻塞，daemon 用空值）；spec 为 YAML，用 serde_yaml 解析填充
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(&raw).unwrap_or(serde_yaml::Value::String(raw.clone()));
    let detect = detect_git();
    if let Some(v) = detect {
        if let serde_yaml::Value::Mapping(map) = &mut value {
            for (k, val) in [("repository", v.0), ("base_revision", v.1)] {
                let key = serde_yaml::Value::String(k.to_string());
                if !map.contains_key(&key) {
                    map.insert(key, serde_yaml::Value::String(val));
                }
            }
        }
    }
    let spec_text = match &value {
        serde_yaml::Value::String(s) => s.clone(),
        _ => serde_yaml::to_string(&value).map_err(|e| CliError::from(e.to_string()))?,
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

/// 打补丁：读取 spec（resolve_spec 目录约定）后替换图定义。
/// 图成本分析（不建图）：解析 + 编译 + 评估，输出"值得/不值得上图"建议。
fn analyze(ctx: &Context, spec: &Path, json: bool) -> Result<(), CliError> {
    let path = resolve_spec(ctx, spec)?;
    let yaml = std::fs::read_to_string(&path).map_err(|e| {
        CliError::new(
            "graph_spec_read_failed",
            format!("读取 spec 失败 {}: {}", path.display(), e),
        )
    })?;
    let spec = crate::graph::spec::GraphSpec::parse(&yaml)
        .map_err(|e| CliError::new("graph_spec_parse_error", format!("spec 解析失败: {e}")))?;
    let compiled = crate::graph::compiler::compile(&spec);
    if !compiled.valid {
        let errs: Vec<String> = compiled.errors.iter().map(|i| i.message.clone()).collect();
        return Err(CliError::new(
            "graph_compile_error",
            format!("spec 编译失败: {}", errs.join("; ")),
        ));
    }
    let cg = compiled
        .compiled
        .as_ref()
        .ok_or_else(|| CliError::new("graph_compile_error", "编译无结果"))?;
    let a = crate::graph::analyze::analyze(cg);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "spec": path.display().to_string(),
                "node_count": a.node_count,
                "longest_chain": a.longest_chain,
                "parallel_factor": a.parallel_factor,
                "writer_nodes": a.writer_nodes,
                "verification_nodes": a.verification_nodes,
                "approval_nodes": a.approval_nodes,
                "est_duration_secs": a.est_duration_secs,
                "verdict": if a.verdict == crate::graph::analyze::AnalysisVerdict::WorthIt { "worth_it" } else { "not_worth_it" },
                "reasons": a.reasons,
            })
        );
        return Ok(());
    }
    println!("spec       : {}", path.display());
    println!("节点数     : {}", a.node_count);
    println!(
        "最长链     : {}（可并行度约 {}）",
        a.longest_chain, a.parallel_factor
    );
    println!("写节点     : {}（worktree 隔离）", a.writer_nodes);
    println!("验证节点   : {}（acceptance 门禁）", a.verification_nodes);
    println!("审批节点   : {}", a.approval_nodes);
    println!("预估时长   : {:.0} 秒（最长链下界）", a.est_duration_secs);
    if a.verdict == crate::graph::analyze::AnalysisVerdict::WorthIt {
        println!("建议       : ✅ 值得上图 —— {}", a.reasons.join("；"));
    } else {
        println!("建议       : ⚠️ 不值得上图（简单任务，走单 agent 更划算，见 docs/graph-engineering-survey.md §5）");
    }
    Ok(())
}

pub fn patch(ctx: &Context, run_id: &str, spec_file: &Path, json: bool) -> Result<(), CliError> {
    let resolved = resolve_spec(ctx, spec_file)?;
    let raw = std::fs::read_to_string(&resolved)
        .map_err(|e| CliError::from(format!("读取 spec 失败 {}: {}", resolved.display(), e)))?;
    let msg = client::post(
        ctx,
        &format!("/api/v1/graph/runs/{run_id}/patch"),
        json!({ "spec": raw }),
    )?;
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
    // repository = 本地 git 仓库根（worktree 需在本地仓库内创建）
    let repo_root = run(&["rev-parse", "--show-toplevel"]);
    let branch =
        run(&["branch", "--show-current"]).or_else(|| run(&["rev-parse", "--short", "HEAD"]));
    match (repo_root, branch) {
        (Some(r), Some(b)) => Some((r, b)),
        _ => None,
    }
}
