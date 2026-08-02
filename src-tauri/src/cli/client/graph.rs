//! 图工程 CLI HTTP 客户端（docs/design_graph.md §6）。

use crate::cli::context::Context;
use crate::proto::ServerMsg;

pub fn delete_graph_run(ctx: &Context, run_id: &str) -> Result<ServerMsg, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/v1/graph/runs/{}", ctx.base_url, run_id);
    let resp = client
        .delete(&url)
        .header("X-AgTalk-Address", &ctx.address)
        .send()
        .map_err(|e| format!("daemon_unreachable: {}（请先 agtalk daemon start）", e))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        if let Ok(ServerMsg::Error { code, message }) = serde_json::from_str(&text) {
            return Err(format!("{}: {}", code, message));
        }
        return Err(format!("http_error: HTTP {}: {}", status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("parse_error: {}", e))
}
