//! `/api/v1/tool/*` handler。

use crate::proto::ServerMsg;
use crate::server::handlers::not_supported;
use crate::server::state::AppState;
use crate::tool::DoctorContext;

pub fn handle_daemon(_state: &AppState, action: Option<String>) -> ServerMsg {
    match action.as_deref() {
        None | Some("status") => {
            let _running = crate::cli::daemon::is_running();
            ServerMsg::Pong
        }
        _ => not_supported("tool daemon action"),
    }
}

pub fn handle_doctor(state: &AppState) -> ServerMsg {
    let ctx = DoctorContext::new(
        state.dot_agtalk.clone(),
        (*state.config).clone(),
        Some(state.storage.clone()),
        None,
    );
    crate::tool::doctor::run(ctx)
}

pub fn handle_version() -> ServerMsg {
    ServerMsg::ToolVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

pub fn handle_path() -> ServerMsg {
    ServerMsg::ToolPathInfo {
        path: std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown".to_string()),
    }
}
