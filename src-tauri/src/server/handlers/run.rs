//! `/api/v1/run` handler。

use crate::proto::ServerMsg;
use crate::run::AuthContext;
use crate::server::state::AppState;
use std::path::PathBuf;

pub fn handle_run(state: &AppState, file: Option<String>, ctx: Option<AuthContext>) -> ServerMsg {
    let file = file.map(PathBuf::from);
    match crate::run::run_file(&state.storage, &state.dot_agtalk, ctx.as_ref(), file) {
        Ok(result) => ServerMsg::RunResult {
            status: result.status,
            file: result.file,
            steps: result.steps,
            stopped_at: result.stopped_at,
        },
        Err(e) => ServerMsg::Error {
            code: "run_failed".into(),
            message: e.to_string(),
        },
    }
}
