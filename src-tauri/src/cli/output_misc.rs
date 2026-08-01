//! daemon/run 输出格式化（cli/output.rs 拆分，控制行数红线）。

use crate::cli::output_doctor::summary_status_label;
use crate::proto::RunStepResult;
use console::Style;

#[allow(clippy::too_many_arguments)]
pub(crate) fn print_daemon_status(
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

pub(crate) fn print_run_report(
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

pub(crate) fn run_step_summary(s: &RunStepResult) -> String {
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

pub(crate) fn format_uptime(seconds: u64) -> String {
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
