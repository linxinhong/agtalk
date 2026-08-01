//! doctor 输出格式化（cli/output.rs 拆分，控制行数红线）。

use crate::proto::{DiagnosisCheck, RootCause};
use console::Style;

pub(crate) fn print_doctor_summary(
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

pub(crate) fn print_doctor_debug_report(status: &str, checks: &[DiagnosisCheck]) {
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

pub(crate) fn status_label(status: &str) -> String {
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

pub(crate) fn summary_status_label(status: &str) -> (String, &'static str) {
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

pub(crate) fn print_doctor_category(category: &str, items: &[&DiagnosisCheck], name_width: usize) {
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

pub(crate) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
