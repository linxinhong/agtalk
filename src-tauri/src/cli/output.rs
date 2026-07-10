//! CLI 输出封装：统一处理文本与 --json 两种模式。

use crate::proto::{DiagnosisCheck, RootCause, RunStepResult, ServerMsg};
use crate::routing::{short_id_of, Message};
use console::Style;
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
        print_doctor_debug_report(status, checks);
    } else {
        print_doctor_summary(status, summary, root_causes, actions, context);
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

fn print_relation_list(relations: &[crate::identity::relations::Relation]) {
    if relations.is_empty() {
        println!("no relations yet");
        return;
    }
    println!(
        "{} peer{}",
        relations.len(),
        if relations.len() == 1 { "" } else { "s" }
    );
    for r in relations {
        let short = crate::routing::short_id_of(&r.address);
        let counts = format!("sent {} / recv {}", r.sent_count, r.received_count);
        println!();
        println!("{}  {}  {}", short, r.name, counts);
        if !r.intro.is_empty() {
            println!("    {}", r.intro);
        }
        if let Some(role) = &r.role {
            println!("    role: {}", role);
        }
        if !r.specialties.is_empty() {
            println!("    specialties: {}", r.specialties.join(", "));
        }
        if !r.preferred_for.is_empty() {
            println!("    preferred_for: {}", r.preferred_for.join(", "));
        }
        if !r.tags.is_empty() {
            println!("    tags: {}", r.tags.join(", "));
        }
    }
}

fn print_relation(relation: &crate::identity::relations::Relation) {
    println!("address       : {}", relation.address);
    println!("name          : {}", relation.name);
    if !relation.intro.is_empty() {
        println!("intro         : {}", relation.intro);
    }
    println!("sent_count    : {}", relation.sent_count);
    println!("received_count: {}", relation.received_count);
    if let Some(first) = relation.first_seen_at {
        println!("first_seen_at : {}", first);
    }
    if let Some(last) = relation.last_seen_at {
        println!("last_seen_at  : {}", last);
    }
    if let Some(last_id) = &relation.last_message_id {
        println!("last_message  : {}", crate::routing::short_id_of(last_id));
    }
    if let Some(role) = &relation.role {
        println!("role          : {}", role);
    }
    if !relation.specialties.is_empty() {
        println!("specialties   : {}", relation.specialties.join(", "));
    }
    if !relation.preferred_for.is_empty() {
        println!("preferred_for : {}", relation.preferred_for.join(", "));
    }
    if !relation.tags.is_empty() {
        println!("tags          : {}", relation.tags.join(", "));
    }
    if let Some(note) = &relation.note {
        println!("note          : {}", note);
    }
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
        } => {
            println!("address   : {}", address);
            println!("name      : {}", name);
            println!("intro     : {}", intro);
            if *notify_ready {
                println!("notify    : {} ready", notify_channel);
            } else {
                println!("notify    : {}", notify_channel);
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
            print_doctor_summary(status, summary, root_causes, actions, context);
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
            print_run_report(status, file.as_deref(), steps, stopped_at);
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
            print_daemon_status(
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
    }
}

fn print_doctor_summary(
    status: &str,
    summary: &str,
    root_causes: &[RootCause],
    actions: &[String],
    context: &crate::proto::DiagnosisContext,
) {
    println!();
    println!("  ╭●─●╮  agtalk doctor  {}", status);
    println!("  ╰─●─╯  {}", summary);

    if !root_causes.is_empty() {
        println!();
        println!("  Root causes:");
        for cause in root_causes {
            let (label, pad) = summary_status_label(&cause.status);
            println!("    {}{}  {}", label, pad, cause.id);
            println!("           {}", cause.message);
        }
    }

    if !actions.is_empty() {
        println!();
        println!("  Actions:");
        for (i, action) in actions.iter().enumerate() {
            println!("    {}. {}", i + 1, action);
        }
    }

    println!();
    println!("  Context:");
    let label_width = 10;
    println!(
        "    {:<width$} {}",
        "identity:",
        context.identity.as_deref().unwrap_or("-"),
        width = label_width
    );
    println!(
        "    {:<width$} {}",
        "address:",
        context.address.as_deref().unwrap_or("-"),
        width = label_width
    );
    let pending_str = match context.pending {
        Some(n) => n.to_string(),
        None => "unavailable".to_string(),
    };
    println!(
        "    {:<width$} {}",
        "pending:",
        pending_str,
        width = label_width
    );

    println!();
    println!("  Debug:");
    println!("    agtalk tool doctor --debug");
    println!("    agtalk --json tool doctor");
}

fn print_doctor_debug_report(status: &str, checks: &[DiagnosisCheck]) {
    let errors = checks.iter().filter(|c| c.status == "error").count();
    let warns = checks.iter().filter(|c| c.status == "warn").count();
    let skips = checks.iter().filter(|c| c.status == "skip").count();

    println!();
    println!("  ╭●─●╮  agtalk doctor  {}", status);
    println!(
        "  ╰─●─╯  {} error{}, {} warning{}, {} skip{}",
        errors,
        if errors == 1 { "" } else { "s" },
        warns,
        if warns == 1 { "" } else { "s" },
        skips,
        if skips == 1 { "" } else { "s" }
    );

    // 按文档约定顺序分节；未定义的分类追加到最后。
    let mut by_category: std::collections::HashMap<&str, Vec<&DiagnosisCheck>> =
        std::collections::HashMap::new();
    for c in checks {
        by_category.entry(&c.category).or_default().push(c);
    }

    let name_width = checks
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(20)
        .max(20);

    let ordered = [
        "runtime", "config", "daemon", "identity", "message", "wait", "notify",
    ];
    let mut seen = std::collections::HashSet::new();
    for category in ordered.iter() {
        if let Some(items) = by_category.get(*category) {
            seen.insert(*category);
            print_doctor_category(category, items.as_slice(), name_width);
        }
    }
    for (category, items) in by_category {
        if seen.insert(category) {
            print_doctor_category(category, items.as_slice(), name_width);
        }
    }
}

fn status_label(status: &str) -> String {
    let green = Style::new().green().bold();
    let red = Style::new().red().bold();
    let yellow = Style::new().yellow().bold();
    let cyan = Style::new().cyan();

    match status {
        "ok" => green.apply_to("  ✓  ").to_string(),
        "error" => red.apply_to("  ✗  ").to_string(),
        "warn" => yellow.apply_to("  ⚠  ").to_string(),
        "info" => cyan.apply_to("  ℹ  ").to_string(),
        "skip" => cyan.apply_to("  -  ").to_string(),
        _ => format!("  {}  ", status),
    }
}

fn summary_status_label(status: &str) -> (String, &'static str) {
    let green = Style::new().green().bold();
    let red = Style::new().red().bold();
    let yellow = Style::new().yellow().bold();

    match status {
        "ok" => (green.apply_to("ok").to_string(), "   "),
        "error" => (red.apply_to("error").to_string(), ""),
        "warn" => (yellow.apply_to("warn").to_string(), " "),
        _ => (status.to_string(), ""),
    }
}

fn print_doctor_category(category: &str, items: &[&DiagnosisCheck], name_width: usize) {
    println!();
    println!("  {}", capitalize(category));
    for c in items {
        let label = status_label(&c.status);
        println!(
            "    {} {:<width$} {}",
            label,
            c.name,
            c.message,
            width = name_width
        );
        if let Some(s) = &c.suggestion {
            println!("          suggestion: {}", s);
        }
        if let Some(cmd) = &c.command {
            println!("          command:    {}", cmd);
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn print_daemon_status(
    pid: u32,
    version: &str,
    http_port: u16,
    uptime_seconds: u64,
    active_mailboxes: i64,
    pending_messages: i64,
    sse_subscribers: usize,
    config_path: &str,
) {
    let blue = Style::new().blue().underlined();

    println!();

    let label_width = 10; // 与最长标签 "Mailboxes:" 对齐

    if pid == 0 {
        println!("  ╭●─●╮  agtalk daemon stopped");
        println!("  ╰─●─╯  Local agent bus is not available on this machine.");
        println!();
        if !config_path.is_empty() {
            println!(
                "  {:<width$} {}",
                "Config:",
                config_path,
                width = label_width
            );
        }
        println!(
            "  {:<width$} agtalk daemon start",
            "Start:",
            width = label_width
        );
        return;
    }

    let title = format!("agtalk daemon running  {}", version);
    println!("  ╭●─●╮  {}", title);
    println!("  ╰─●─╯  Local agent bus is available on this machine.");
    println!();

    let local_url = format!("http://127.0.0.1:{}", http_port);
    println!(
        "  {:<width$} {}",
        "Local:",
        blue.apply_to(local_url),
        width = label_width
    );
    if !config_path.is_empty() {
        println!(
            "  {:<width$} {}",
            "Config:",
            config_path,
            width = label_width
        );
    }
    println!("  {:<width$} {}", "PID:", pid, width = label_width);
    println!(
        "  {:<width$} {}",
        "Uptime:",
        format_uptime(uptime_seconds),
        width = label_width
    );
    println!(
        "  {:<width$} {} active",
        "Mailboxes:",
        active_mailboxes,
        width = label_width
    );
    println!(
        "  {:<width$} {} messages",
        "Pending:",
        pending_messages,
        width = label_width
    );
    println!(
        "  {:<width$} {}",
        "SSE subs:",
        sse_subscribers,
        width = label_width
    );
    println!(
        "  {:<width$} agtalk daemon stop",
        "Stop:",
        width = label_width
    );
}

fn print_run_report(
    status: &str,
    file: Option<&str>,
    steps: &[RunStepResult],
    stopped_at: &Option<usize>,
) {
    println!();
    if let Some(file) = file {
        println!("run: {}", file);
    }
    if !steps.is_empty() {
        println!();
    }
    for s in steps {
        let (label, pad) = summary_status_label(&s.status);
        println!("    {}{}  {:>3}  {}", label, pad, s.index, s.action);
        let summary = run_step_summary(s);
        if !summary.is_empty() {
            println!("               {}", summary);
        }
    }
    if let Some(step) = stopped_at {
        println!();
        println!("stopped at step {}", step);
    } else if status != "ok" && !steps.is_empty() {
        println!();
        println!("status: {}", status);
    }
}

fn run_step_summary(s: &RunStepResult) -> String {
    if let Some(ref e) = s.error {
        return format!("{}: {}", e.code, e.message);
    }
    if let Some(id) = s.output.get("id").and_then(|v| v.as_str()) {
        return id.to_string();
    }
    if let Some(ty) = s.output.get("type").and_then(|v| v.as_str()) {
        return ty.to_string();
    }
    String::new()
}

fn format_uptime(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{}s", seconds);
    }
    let mins = seconds / 60;
    let secs = seconds % 60;
    if mins < 60 {
        return format!("{}m {}s", mins, secs);
    }
    let hours = mins / 60;
    let mins = mins % 60;
    if hours < 24 {
        return format!("{}h {}m {}s", hours, mins, secs);
    }
    let days = hours / 24;
    let hours = hours % 24;
    format!("{}days {}h {}m {}s", days, hours, mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_left_text_format() {
        let msg = ServerMsg::IdentityLeft {
            address: "550e8400-e29b-41d4-a716-446655440000".into(),
            name: "reviewer".into(),
            removed_session: true,
            purge: true,
        };
        print_text_server_msg(&msg);
    }

    #[test]
    fn inbox_text_uses_short_id_and_summary() {
        let msg = ServerMsg::InboxResult {
            messages: vec![Message {
                id: "6f0d4353-f4b2-46ab-afb6-8c7f6af02049".into(),
                to_address: "to".into(),
                to_name: "to".into(),
                from_address: "from".into(),
                from_name: "notify-receiver-zellij".into(),
                body: "test notify via zellij plugin".into(),
                content_type: "text".into(),
                reply_to_id: None,
                metadata: "{}".into(),
                event_id: 1,
                status: "read".into(),
                created_at: 0.0,
            }],
        };
        print_text_server_msg(&msg);
    }

    #[test]
    fn msg_detail_text_uses_summary() {
        let msg = ServerMsg::MsgDetail(Message {
            id: "6f0d4353-f4b2-46ab-afb6-8c7f6af02049".into(),
            to_address: "to".into(),
            to_name: "to".into(),
            from_address: "from".into(),
            from_name: "notify-receiver-zellij".into(),
            body: "test notify via zellij plugin".into(),
            content_type: "text".into(),
            reply_to_id: None,
            metadata: "{}".into(),
            event_id: 1,
            status: "read".into(),
            created_at: 0.0,
        });
        print_text_server_msg(&msg);
    }

    #[test]
    fn identity_ambiguous_json_has_candidates() {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "candidates".to_string(),
            serde_json::json!(["nora", "quinn"]),
        );
        let e = CliError::new("identity_ambiguous", "多个 session").with_extra(extra);
        let json = serde_json::to_string(&JsonError {
            ty: "error",
            code: &e.code,
            message: &e.message,
            extra: e.extra.as_ref(),
        })
        .unwrap();
        assert!(json.contains("identity_ambiguous"));
        assert!(json.contains("candidates"));
        assert!(json.contains("nora"));
    }

    #[test]
    fn format_uptime_seconds() {
        assert_eq!(format_uptime(11), "11s");
    }

    #[test]
    fn format_uptime_minutes() {
        assert_eq!(format_uptime(611), "10m 11s");
    }

    #[test]
    fn format_uptime_hours() {
        assert_eq!(format_uptime(43811), "12h 10m 11s");
    }

    #[test]
    fn format_uptime_days() {
        assert_eq!(format_uptime(993011), "11days 11h 50m 11s");
    }

    #[test]
    fn format_uptime_one_day_one_second() {
        assert_eq!(format_uptime(86401), "1days 0h 0m 1s");
    }
}
