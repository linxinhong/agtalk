//! CLI 输出封装：统一处理文本与 --json 两种模式。

use crate::proto::ServerMsg;
use crate::routing::{short_id_of, Message};
use serde::Serialize;
use std::process::ExitCode;

#[derive(Debug, Clone)]
pub struct CliError {
    pub code: String,
    pub message: String,
    /// --json 输出时附加的字段，如 identity_ambiguous 的 candidates。
    pub extra: Option<serde_json::Map<String, serde_json::Value>>,
}

impl CliError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            extra: None,
        }
    }

    pub fn with_extra(mut self, extra: serde_json::Map<String, serde_json::Value>) -> Self {
        self.extra = Some(extra);
        self
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self {
            code: "error".to_string(),
            message,
            extra: None,
        }
    }
}

impl From<crate::cli::context_error::IdentityResolutionError> for CliError {
    fn from(e: crate::cli::context_error::IdentityResolutionError) -> Self {
        let code = e.code().to_string();
        let message = e.to_string();
        let mut extra = serde_json::Map::new();
        if let crate::cli::context_error::IdentityResolutionError::Ambiguous(names) = e {
            extra.insert(
                "candidates".to_string(),
                serde_json::Value::Array(
                    names.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }
        Self {
            code,
            message,
            extra: if extra.is_empty() { None } else { Some(extra) },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct JsonError<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    code: &'a str,
    message: &'a str,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    extra: Option<&'a serde_json::Map<String, serde_json::Value>>,
}

/// 运行一个命令并打印输出；--json 模式下错误输出到 stderr。
pub fn run_with_output<F>(json: bool, f: F) -> ExitCode
where
    F: FnOnce() -> Result<(), CliError>,
{
    match f() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                let err = JsonError {
                    ty: "error",
                    code: &e.code,
                    message: &e.message,
                    extra: e.extra.as_ref(),
                };
                eprintln!("{}", serde_json::to_string(&err).unwrap_or_default());
            } else {
                print_text_error(&e);
            }
            ExitCode::FAILURE
        }
    }
}

fn print_text_error(e: &CliError) {
    eprintln!("{}: {}", e.code, e.message);
    if let Some(extra) = &e.extra {
        if let Some(candidates) = extra.get("candidates").and_then(|v| v.as_array()) {
            eprintln!();
            eprintln!("Candidates:");
            for c in candidates {
                if let Some(name) = c.as_str() {
                    eprintln!("  - {}", name);
                }
            }
            eprintln!();
            eprintln!("Next:");
            eprintln!("  agtalk --as <name> id show");
            eprintln!("  agtalk --as <name> msg read");
            eprintln!("  agtalk id lookup");
        }
    }
}

/// 打印成功响应；--json 下序列化为 JSON，否则用默认文本格式。
use super::{output_doctor, output_misc};

pub fn print_server_msg(json: bool, msg: &ServerMsg) {
    if json {
        println!("{}", serde_json::to_string(msg).unwrap_or_default());
    } else {
        print_text_server_msg(msg);
    }
}

/// 打印 doctor 诊断结果；支持 --json / --debug 两种额外模式。
pub fn print_doctor_msg(json: bool, debug: bool, msg: &ServerMsg) {
    if json {
        println!("{}", serde_json::to_string(msg).unwrap_or_default());
        return;
    }
    let ServerMsg::ToolDiagnosis {
        status,
        summary,
        root_causes,
        actions,
        context,
        checks,
    } = msg
    else {
        print_text_server_msg(msg);
        return;
    };
    if debug {
        output_doctor::print_doctor_debug_report(status, checks);
    } else {
        output_doctor::print_doctor_summary(status, summary, root_causes, actions, context);
    }
}

fn print_inbox(messages: &[Message]) {
    let count = messages.len();
    println!("{} message{}", count, if count == 1 { "" } else { "s" });
    if messages.is_empty() {
        return;
    }
    println!();
    for (idx, m) in messages.iter().enumerate() {
        print_message_summary(m, Some(idx + 1));
        if idx + 1 < messages.len() {
            println!();
        }
    }
}

fn print_message_summary(m: &Message, index: Option<usize>) {
    let short = short_id_of(&m.id);
    let prefix = index.map(|i| format!("[{}] ", i)).unwrap_or_default();
    println!("{}{}  from {}", prefix, short, m.from_name);
    println!(
        "    type: {}    status: {}    event: {}",
        m.content_type, m.status, m.event_id
    );
    if let Some(subject) = &m.subject {
        println!("    subject: {}", subject);
    }
    println!("    reply: agtalk msg reply {} \"<body>\"", short);
    println!("    done:  agtalk msg done {}", short);
    println!();
    println!("{}", m.body);
}

fn print_wait_result(messages: &[Message], body: &str) {
    if let Some(first) = messages.first() {
        print_message_summary(first, None);
    } else {
        println!("{}", body);
    }
}

fn format_relation_list(relations: &[crate::identity::relations::Relation]) -> String {
    let mut lines: Vec<String> = Vec::new();
    if relations.is_empty() {
        lines.push("no relations yet".to_string());
        return lines.join("\n");
    }
    lines.push(format!(
        "{} peer{}",
        relations.len(),
        if relations.len() == 1 { "" } else { "s" }
    ));
    for r in relations {
        let short = crate::routing::short_id_of(&r.address);
        let counts = format!("sent {} / recv {}", r.sent_count, r.received_count);
        lines.push(String::new());
        lines.push(format!("{}  {}  {}", short, r.name, counts));
        if !r.intro.is_empty() {
            lines.push(format!("    {}", r.intro));
        }
        if let Some(role) = &r.role {
            lines.push(format!("    role: {}", role));
        }
        if !r.specialties.is_empty() {
            lines.push(format!("    specialties: {}", r.specialties.join(", ")));
        }
        if !r.preferred_for.is_empty() {
            lines.push(format!("    preferred_for: {}", r.preferred_for.join(", ")));
        }
        if !r.tags.is_empty() {
            lines.push(format!("    tags: {}", r.tags.join(", ")));
        }
    }
    lines.join("\n")
}

fn print_relation_list(relations: &[crate::identity::relations::Relation]) {
    println!("{}", format_relation_list(relations));
}

fn format_relation(relation: &crate::identity::relations::Relation) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("address       : {}", relation.address));
    lines.push(format!("name          : {}", relation.name));
    if !relation.intro.is_empty() {
        lines.push(format!("intro         : {}", relation.intro));
    }
    lines.push(format!("sent_count    : {}", relation.sent_count));
    lines.push(format!("received_count: {}", relation.received_count));
    if let Some(first) = relation.first_seen_at {
        lines.push(format!("first_seen_at : {}", first));
    }
    if let Some(last) = relation.last_seen_at {
        lines.push(format!("last_seen_at  : {}", last));
    }
    if let Some(last_id) = &relation.last_message_id {
        lines.push(format!(
            "last_message  : {}",
            crate::routing::short_id_of(last_id)
        ));
    }
    if let Some(role) = &relation.role {
        lines.push(format!("role          : {}", role));
    }
    if !relation.specialties.is_empty() {
        lines.push(format!(
            "specialties   : {}",
            relation.specialties.join(", ")
        ));
    }
    if !relation.preferred_for.is_empty() {
        lines.push(format!(
            "preferred_for : {}",
            relation.preferred_for.join(", ")
        ));
    }
    if !relation.tags.is_empty() {
        lines.push(format!("tags          : {}", relation.tags.join(", ")));
    }
    if let Some(note) = &relation.note {
        lines.push(format!("note          : {}", note));
    }
    lines.join("\n")
}

fn print_relation(relation: &crate::identity::relations::Relation) {
    println!("{}", format_relation(relation));
}

fn print_cleanup_result(
    dry_run: bool,
    removed: &[crate::proto::CleanupItem],
    skipped: &[crate::proto::CleanupItem],
) {
    if dry_run {
        println!("dry run: the following items would be removed");
    } else {
        let removed_count = removed.len();
        println!(
            "removed {} item{}",
            removed_count,
            if removed_count == 1 { "" } else { "s" }
        );
    }

    if !removed.is_empty() {
        println!();
        for item in removed {
            println!("  - {} ({}): {}", item.name, item.address, item.reason);
        }
    }

    if !skipped.is_empty() {
        println!();
        if dry_run {
            println!("skipped (would remain active):");
        } else {
            println!("skipped:");
        }
        for item in skipped {
            println!("  - {} ({}): {}", item.name, item.address, item.reason);
        }
    }
}

fn print_text_server_msg(msg: &ServerMsg) {
    match msg {
        ServerMsg::Pong => println!("pong"),
        ServerMsg::Ok { id } => println!("{}", id),
        ServerMsg::Error { code, message } => eprintln!("{}: {}", code, message),
        ServerMsg::AgentHelp { text, .. } => println!("{}", text),
        ServerMsg::AgentGuide { markdown } => println!("{}", markdown),
        ServerMsg::Identity {
            address,
            name,
            intro,
            notify_channel,
            notify_ready,
            workspace_root,
            notify_diagnostics,
        } => {
            println!("address   : {}", address);
            println!("name      : {}", name);
            println!("intro     : {}", intro);
            println!("workspace : {}", workspace_root);
            println!("session   : {}/{}/session.json", workspace_root, name);
            if *notify_ready {
                println!("notify    : {} ready", notify_channel);
            } else {
                println!("notify    : {}", notify_channel);
                for probe in notify_diagnostics {
                    println!(
                        "probe     : {} {} - {}",
                        probe.name, probe.status, probe.message
                    );
                }
                println!(
                    "hint      : rejoin inside zellij/tmux or install a notify plugin to enable notifications"
                );
            }
        }
        ServerMsg::IdentityLeft {
            address,
            name,
            removed_session,
            purge,
        } => {
            println!("left: {} {}", name, address);
            if *purge && !removed_session {
                println!("warning: session directory could not be removed");
            }
        }
        ServerMsg::LookupResult { mailboxes } => {
            for mb in mailboxes {
                println!(
                    "{}\t{}\tnotify={}\t{}",
                    mb.address, mb.name, mb.notify, mb.intro
                );
            }
        }
        ServerMsg::CleanupResult {
            dry_run,
            removed,
            skipped,
        } => {
            print_cleanup_result(*dry_run, removed, skipped);
        }
        ServerMsg::InboxResult { messages } => {
            print_inbox(messages);
        }
        ServerMsg::MsgDetail(m) => {
            print_message_summary(m, None);
        }
        ServerMsg::WaitResult { messages, body } => {
            print_wait_result(messages, body);
        }
        ServerMsg::AskResult { message_id } => {
            println!("{}", message_id);
        }
        ServerMsg::MemPlanShow {
            address,
            name,
            plan,
            context,
            status,
            summary,
            updated_at,
        } => {
            println!("address    : {}", address);
            println!("name       : {}", name);
            println!("updated_at : {}", updated_at);
            println!("status     : {}", status);
            println!("summary    : {}", summary);
            println!("\n--- plan ---\n{}", plan);
            println!("\n--- context ---\n{}", context);
        }
        ServerMsg::MemPlanStatus {
            address,
            name,
            updated_at,
            status,
            summary,
        } => {
            println!("address    : {}", address);
            println!("name       : {}", name);
            println!("updated_at : {}", updated_at);
            println!("status     : {}", status);
            println!("summary    : {}", summary);
        }
        ServerMsg::MemPackResult { topic, entries } => {
            println!("topic: {}", topic);
            for e in entries {
                println!("{}", serde_json::to_string_pretty(e).unwrap_or_default());
            }
        }
        ServerMsg::MemPack { topic, markdown } => {
            println!("# {}\n\n{}", topic, markdown);
        }
        ServerMsg::MemSearchResult { entries } => {
            for e in entries {
                println!("{}", serde_json::to_string_pretty(e).unwrap_or_default());
            }
        }
        ServerMsg::MemShowResult { entry } => {
            println!(
                "{}",
                serde_json::to_string_pretty(entry).unwrap_or_default()
            );
        }
        ServerMsg::MemRelationList { relations } => {
            print_relation_list(relations);
        }
        ServerMsg::MemRelation { relation } => {
            print_relation(relation);
        }
        ServerMsg::ConfigValue { key, value } => {
            println!("{} = {}", key, value);
        }
        ServerMsg::ConfigShowResult { config } => {
            println!(
                "{}",
                serde_json::to_string_pretty(config).unwrap_or_default()
            );
        }
        ServerMsg::ConfigPath { path } => {
            println!("{}", path);
        }
        ServerMsg::ToolDiagnosis {
            status,
            summary,
            root_causes,
            actions,
            context,
            ..
        } => {
            // 非 doctor 命令路径的兜底：默认摘要模式。
            output_doctor::print_doctor_summary(status, summary, root_causes, actions, context);
        }
        ServerMsg::ToolVersionInfo { version } => {
            println!("{}", version);
        }
        ServerMsg::ToolPathInfo { path } => {
            println!("{}", path);
        }
        ServerMsg::RunResult {
            status,
            file,
            steps,
            stopped_at,
        } => {
            output_misc::print_run_report(status, file.as_deref(), steps, stopped_at);
        }
        ServerMsg::DaemonStatus {
            pid,
            version,
            http_port,
            uptime_seconds,
            active_mailboxes,
            pending_messages,
            sse_subscribers,
            config_path,
            db_path: _,
            ..
        } => {
            output_misc::print_daemon_status(
                *pid,
                version,
                *http_port,
                *uptime_seconds,
                *active_mailboxes,
                *pending_messages,
                *sse_subscribers,
                config_path,
            );
        }
        ServerMsg::BrowserJoinResult {
            address,
            name,
            token,
        } => {
            println!("address   : {}", address);
            println!("name      : {}", name);
            println!("token     : {}", token);
        }
        // graph（图工程，docs/design_graph.md §7）
        ServerMsg::GraphRunCreated {
            run_id,
            status,
            errors,
            warnings,
        } => {
            if run_id.is_empty() {
                println!("status    : {}", status);
            } else {
                println!("run_id    : {}", run_id);
                println!("status    : {}", status);
            }
            for e in errors {
                eprintln!("error     : {} - {}", e.code, e.message);
            }
            if !warnings.is_empty() {
                for w in warnings {
                    println!("warning   : {} - {}", w.code, w.message);
                }
            }
        }
        ServerMsg::GraphRunList { runs } => {
            if runs.is_empty() {
                println!("no graph runs");
            }
            for r in runs {
                println!(
                    "{}  {}  {}  {}",
                    short_id(&r.id),
                    r.status,
                    r.goal,
                    r.created_at
                );
            }
        }
        ServerMsg::GraphRunDetail {
            run,
            nodes,
            edges,
            required_approvals,
            resource_conflicts,
        } => {
            println!("run_id    : {}", run.id);
            println!("goal      : {}", run.goal);
            println!("status    : {}", run.status);
            if let Some(repo) = &run.repository {
                println!("repository: {}", repo);
            }
            if let Some(base) = &run.base_revision {
                println!("base      : {}", base);
            }
            println!("nodes     : {}", nodes.len());
            for n in nodes {
                println!(
                    "  [{}] {} ({}) attempt={}",
                    n.status, n.node_key, n.node_type, n.attempt
                );
                if let Some(f) = &n.failure_detail {
                    println!("          failure: {}", f);
                }
            }
            println!("edges     : {}", edges.len());
            if !required_approvals.is_empty() {
                println!("approvals : {}", required_approvals.join(", "));
            }
            if !resource_conflicts.is_empty() {
                println!("conflicts : {}", resource_conflicts.len());
            }
        }
        ServerMsg::GraphEventsResult { events } => {
            for e in events {
                println!(
                    "#{} {} {}{}",
                    e.id,
                    e.event_type,
                    e.node_key.as_deref().unwrap_or("-"),
                    if e.payload.is_null() {
                        String::new()
                    } else {
                        format!(" {}", e.payload)
                    }
                );
            }
        }
        ServerMsg::GraphNodeReportOk {
            run_id,
            node_key,
            attempt,
            status,
            message,
        } => {
            println!("status    : {}", status);
            println!("run_id    : {}", run_id);
            println!("node      : {} (attempt {})", node_key, attempt);
            println!("message   : {}", message);
        }
        ServerMsg::GraphRunControlOk {
            run_id,
            action,
            status,
        } => {
            println!("run_id    : {}", run_id);
            println!("action    : {}", action);
            println!("status    : {}", status);
        }
    }
}

fn short_id(id: &str) -> &str {
    if id.len() > 8 {
        &id[..8]
    } else {
        id
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
