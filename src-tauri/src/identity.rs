//! 身份解析：--as > AGTALK_NAME（兼容 AGTALK_AGENT_NAME） > 单 active session > 报错。

use anyhow::{bail, Result};

use crate::{session, workspace};

pub struct ResolvedIdentity {
    #[allow(dead_code)]
    pub workspace_id: String,
    pub participant_name: String,
    pub session_id: String,
    pub token: String,
    pub socket: String,
}

/// 按优先级解析身份。
pub fn resolve_identity(as_arg: Option<&str>) -> Result<ResolvedIdentity> {
    let name: Option<String> = if let Some(a) = as_arg {
        let a = a.trim();
        if a.is_empty() {
            None
        } else {
            Some(a.to_string())
        }
    } else {
        std::env::var("AGTALK_NAME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .or_else(|| {
                // 兼容旧 AGTALK_AGENT_NAME
                std::env::var("AGTALK_AGENT_NAME")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| {
                        eprintln!("[agtalk] 警告: AGTALK_AGENT_NAME 已弃用，请改用 AGTALK_NAME");
                        s.trim().to_string()
                    })
            })
    };

    let name = match name {
        Some(n) => n,
        None => {
            // 自动：要求恰好 1 个 active session
            let actives = session::list_active_sessions()?;
            match actives.len() {
                0 => bail!(
                    "无 active session，请先 `agtalk join <name>`，或用 --as / AGTALK_NAME 指定"
                ),
                1 => actives[0].clone(),
                _ => bail!(
                    "有 {} 个 active session，请用 --as <name> 或 AGTALK_NAME 指定: {}",
                    actives.len(),
                    actives.join(", ")
                ),
            }
        }
    };

    let sf =
        session::read_session(&name)?.ok_or_else(|| anyhow::anyhow!("session 不存在: {}", name))?;
    if sf.session.status != "active" {
        bail!("session {} 状态为 {}，请重新 join", name, sf.session.status);
    }

    let wf = workspace::read_workspace()?
        .ok_or_else(|| anyhow::anyhow!("未找到 workspace.json，请先 `agtalk join`"))?;

    let socket = wf.daemon.socket.unwrap_or_else(crate::paths::socket_path);

    Ok(ResolvedIdentity {
        workspace_id: wf.workspace.id,
        participant_name: sf.name,
        session_id: sf.session.id,
        token: sf.session.token,
        socket,
    })
}
