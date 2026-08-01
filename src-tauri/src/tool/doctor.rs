//! `agtalk tool doctor` 实现。

use crate::proto::{DiagnosisCheck, RootCause, ServerMsg};
use crate::tool::doctor_checks::ResolvedIdentity;
use crate::tool::DoctorContext;

/// 运行 doctor 检查。
pub fn run(ctx: DoctorContext) -> ServerMsg {
    let mut checks = Vec::new();

    checks.extend(crate::tool::doctor_checks::runtime_checks());
    checks.extend(crate::tool::doctor_checks::config_checks(&ctx.config));
    checks.extend(crate::tool::doctor_checks::feishu_checks(&ctx.config));
    checks.extend(crate::tool::doctor_checks::daemon_checks(&ctx));

    let identity = crate::tool::doctor_identity::resolve_identity_readonly(&ctx);
    checks.extend(crate::tool::doctor_identity::identity_checks(
        &ctx, &identity,
    ));
    checks.extend(crate::tool::doctor_identity::local_state_checks(
        &ctx, &identity,
    ));
    checks.extend(crate::tool::doctor_message::message_checks(&ctx, &identity));
    checks.extend(crate::tool::doctor_message::wait_checks(&ctx, &identity));
    checks.extend(crate::tool::doctor_message::notify_checks(&ctx, &identity));

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

pub(crate) fn aggregate_status(checks: &[DiagnosisCheck]) -> String {
    if checks.iter().any(|c| c.status == "error") {
        "error".to_string()
    } else if checks.iter().any(|c| c.status == "warn") {
        "warn".to_string()
    } else {
        "ok".to_string()
    }
}

pub(crate) fn compute_summary(status: &str, checks: &[DiagnosisCheck]) -> String {
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

pub(crate) fn has_error(checks: &[DiagnosisCheck], name: &str) -> bool {
    checks.iter().any(|c| c.name == name && c.status == "error")
}

pub(crate) fn compute_root_causes(
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

pub(crate) fn compute_actions(
    root_causes: &[RootCause],
    identity: &Option<ResolvedIdentity>,
) -> Vec<String> {
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

pub(crate) fn compute_context(
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

pub(crate) fn check(
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

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod tests;
