//! doctor 身份/本地状态检查（doctor.rs 拆分，控制行数红线）。
use crate::identity::agents_map;
use crate::identity::relations;
use crate::identity::{mailbox, session_file};
use crate::proto::DiagnosisCheck;
use crate::tool::doctor::*;
use crate::tool::doctor_checks::*;
use crate::tool::DoctorContext;
use std::path::Path;
use sysinfo::{Pid, System};
pub(crate) fn resolve_identity_readonly(ctx: &DoctorContext) -> Option<ResolvedIdentity> {
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

pub(crate) fn find_registered_ancestor(
    dot: &Path,
) -> Result<Option<agents_map::AgentEntry>, String> {
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

pub(crate) fn os_process_start_time(sys: &mut System, pid: u32) -> u64 {
    sys.refresh_processes();
    sys.process(Pid::from(pid as usize))
        .map(|p| p.start_time())
        .unwrap_or(0)
}

pub(crate) fn list_session_names(dot: &Path) -> Result<Vec<String>, String> {
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

pub(crate) fn identity_checks(
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
pub(crate) fn local_state_checks(
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
pub(crate) fn path_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
pub(crate) fn path_mode(_path: &Path) -> Option<u32> {
    Some(0o600)
}

// ---- message ----
