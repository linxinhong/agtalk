//! Tauri 命令桥（薄）。
//! 每个 #[tauri::command] 只做：接收前端参数 → 调对应领域函数 → 返回结果。
//! popup 窗口的业务全部经 HumanClient（human/client.rs）走 daemon API，
//! token 留在 Rust 侧，不下发到前端。

use crate::human::client::HumanClient;
use crate::human::popup::POPUP_SURFACE;
use crate::proto::ServerMsg;
use crate::routing::Message;
use serde::Serialize;
use tauri::Emitter;

/// popup 窗口状态：本窗口对应的消息 id（由 `agtalk __popup <msg-id>` 传入）。
pub struct PopupState {
    pub message_id: String,
}

/// popup 展示模型：消息详情 + human 地址（前端展示用）。
#[derive(Debug, Serialize)]
pub struct PopupView {
    pub message: Message,
    pub human_address: String,
}

/// 加载消息：标 read 取详情，并回执 delivery（ack 失败只记录，不影响展示）。
pub(crate) fn load_view(message_id: &str) -> Result<PopupView, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    let human_address = client.address().to_string();
    let message = client.read(message_id).map_err(|e| e.to_string())?;
    if let Err(e) = client.delivery_ack(POPUP_SURFACE, message_id) {
        tracing::warn!("popup delivery ack failed: {}", e);
    }
    Ok(PopupView {
        message,
        human_address,
    })
}

/// 回复消息；choice 用于审批选项。返回回复消息 id。
pub(crate) fn reply_message(
    message_id: &str,
    body: &str,
    choices: &[String],
) -> Result<String, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .reply(POPUP_SURFACE, message_id, body, choices)
        .map_err(|e| e.to_string())
}

/// 标记消息完成。
pub(crate) fn done_message(message_id: &str) -> Result<(), String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .done(POPUP_SURFACE, message_id)
        .map_err(|e| e.to_string())
}

/// 取消消息：给原发送方回「（已取消）」并终结原消息。
pub(crate) fn cancel_message(message_id: &str) -> Result<String, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .cancel(POPUP_SURFACE, message_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn popup_load(state: tauri::State<'_, PopupState>) -> Result<PopupView, String> {
    load_view(&state.message_id)
}

#[tauri::command]
pub fn popup_reply(
    state: tauri::State<'_, PopupState>,
    body: String,
    choices: Vec<String>,
) -> Result<String, String> {
    reply_message(&state.message_id, &body, &choices)
}

#[tauri::command]
pub fn popup_done(state: tauri::State<'_, PopupState>) -> Result<(), String> {
    done_message(&state.message_id)
}

#[tauri::command]
pub fn popup_cancel(state: tauri::State<'_, PopupState>) -> Result<String, String> {
    cancel_message(&state.message_id)
}

// ---- GUI 主窗口（配置界面）----
// 薄客户端：配置读写经 daemon HTTP API，与 CLI config show/set 同源；
// config 端点本机免认证（127.0.0.1 单用户威胁模型）。

/// GUI 配置视图：完整配置 JSON + 配置文件路径（前端展示用）。
#[derive(Debug, Serialize)]
pub struct GuiConfigView {
    pub config: serde_json::Value,
    pub path: String,
}

fn gui_base_url() -> Result<String, String> {
    let cfg = crate::config::AgConfig::load().map_err(|e| e.to_string())?;
    Ok(format!("http://127.0.0.1:{}", cfg.http_port))
}

/// 加载配置视图（GUI 主窗口入口）。
pub(crate) fn load_config_view() -> Result<GuiConfigView, String> {
    let base = gui_base_url()?;
    load_config_view_from(&base)
}

/// 保存单个点分配置项（GUI 主窗口入口）。
pub(crate) fn set_config_value(key: &str, value: &str) -> Result<(), String> {
    let base = gui_base_url()?;
    set_config_value_to(&base, key, value)
}

fn load_config_view_from(base: &str) -> Result<GuiConfigView, String> {
    let config = match gui_request(reqwest::Method::GET, base, "/api/v1/config", None)? {
        ServerMsg::ConfigShowResult { config } => config,
        ServerMsg::Error { code, message } => return Err(format!("{}: {}", code, message)),
        other => return Err(format!("unexpected_response: {:?}", other)),
    };
    let path = match gui_request(reqwest::Method::GET, base, "/api/v1/config/path", None)? {
        ServerMsg::ConfigPath { path } => path,
        ServerMsg::Error { code, message } => return Err(format!("{}: {}", code, message)),
        other => return Err(format!("unexpected_response: {:?}", other)),
    };
    Ok(GuiConfigView { config, path })
}

fn set_config_value_to(base: &str, key: &str, value: &str) -> Result<(), String> {
    let endpoint = format!("/api/v1/config/{}", key);
    let body = serde_json::json!({ "value": value });
    match gui_request(reqwest::Method::PATCH, base, &endpoint, Some(body))? {
        ServerMsg::Pong => Ok(()),
        ServerMsg::Error { code, message } => Err(format!("{}: {}", code, message)),
        other => Err(format!("unexpected_response: {:?}", other)),
    }
}

/// 图工程命令桥：daemon 认证用 human token（X-AgTalk-Human-Token，design_graph.md §1.2）。
/// token 只在本进程内使用，绝不下发前端。
fn graph_request(
    method: reqwest::Method,
    base: &str,
    endpoint: &str,
    body: Option<serde_json::Value>,
) -> Result<ServerMsg, String> {
    let session =
        crate::identity::human_session::load().map_err(|e| format!("human session 不可用: {e}"))?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}{}", base, endpoint);
    let mut builder = client
        .request(method, &url)
        .header(reqwest::header::AUTHORIZATION.as_str(), "");
    builder = builder.header("X-AgTalk-Human-Token", session.token);
    if let Some(b) = body {
        builder = builder.json(&b);
    }
    let resp = builder
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

/// GraphRun 列表（图工程管理界面顶部）。
#[tauri::command]
pub fn gui_graph_list(status: Option<String>) -> Result<serde_json::Value, String> {
    let base = gui_base_url()?;
    let endpoint = match status {
        Some(s) => format!("/api/v1/graph/runs?status={}", s),
        None => "/api/v1/graph/runs".to_string(),
    };
    let msg = graph_request(reqwest::Method::GET, &base, &endpoint, None)?;
    serde_json::to_value(msg).map_err(|e| e.to_string())
}

/// GraphRun 详情（画布数据）。
#[tauri::command]
pub fn gui_graph_show(run_id: String) -> Result<serde_json::Value, String> {
    let base = gui_base_url()?;
    let msg = graph_request(
        reqwest::Method::GET,
        &base,
        &format!("/api/v1/graph/runs/{}", run_id),
        None,
    )?;
    serde_json::to_value(msg).map_err(|e| e.to_string())
}

/// GraphRun 事件日志（since 之后）。
#[tauri::command]
pub fn gui_graph_events(run_id: String, since: Option<i64>) -> Result<serde_json::Value, String> {
    let base = gui_base_url()?;
    let endpoint = format!(
        "/api/v1/graph/runs/{}/events{}",
        run_id,
        since.map(|s| format!("?since={}", s)).unwrap_or_default()
    );
    let msg = graph_request(reqwest::Method::GET, &base, &endpoint, None)?;
    serde_json::to_value(msg).map_err(|e| e.to_string())
}

/// 提交图 spec（human 允许，design_graph.md §6）。
#[tauri::command]
pub fn gui_graph_submit(spec: String) -> Result<serde_json::Value, String> {
    let base = gui_base_url()?;
    let msg = graph_request(
        reqwest::Method::POST,
        &base,
        "/api/v1/graph/submit",
        Some(serde_json::json!({ "spec": spec })),
    )?;
    serde_json::to_value(msg).map_err(|e| e.to_string())
}

/// 节点"复制接管提示词"（Tim 设计稿方案二）：查 participant → 读本地 session → 渲染接管文本。
/// 薄桥：只做转发与组装，渲染逻辑在 identity/prompt.rs（与 CLI id prompt 单一事实来源）。
#[tauri::command]
pub fn gui_node_prompt(run_id: String, node_key: String) -> Result<String, String> {
    let base = gui_base_url()?;
    let msg = graph_request(
        reqwest::Method::GET,
        &base,
        &format!("/api/v1/graph/runs/{}", run_id),
        None,
    )?;
    let crate::proto::ServerMsg::GraphRunDetail { nodes, .. } = msg else {
        return Err("无法读取图详情".into());
    };
    let node = nodes
        .iter()
        .find(|n| n.node_key == node_key)
        .ok_or_else(|| format!("节点 {node_key} 不存在"))?;
    let participant: &str = node
        .participant_id
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            String::from("该节点无外部执行者（deterministic/join/gate/approval 无需接管提示词）")
        })?;
    // 读 participant 的本地 session（GUI 启动目录的 .agtalk/<name>/session.json）
    let ctx =
        crate::cli::context::Context::pre_join().map_err(|e| format!("无法定位 workspace: {e}"))?;
    let session =
        crate::identity::session_file::read(&ctx.dot_agtalk, participant).map_err(|e| {
            format!(
                "participant '{participant}' 的 session 不在当前 workspace（{}）: {e}",
                ctx.dot_agtalk.display()
            )
        })?;
    let text = crate::identity::prompt::render_onboarding_prompt(&session);
    // 剪贴板写入走 Rust 侧（arboard/NSPasteboard）：WKWebView 的 navigator.clipboard
    // 可能受限（焦点/secure context），且不新增 capability——自定义命令即可。
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| format!("剪贴板不可用: {e}（应用需在前台有焦点）"))?;
    cb.set_text(text.clone())
        .map_err(|e| format!("写入剪贴板失败: {e}（应用可能失焦）"))?;
    Ok(text)
}

/// 取消运行。
#[tauri::command]
pub fn gui_graph_cancel(run_id: String) -> Result<serde_json::Value, String> {
    let base = gui_base_url()?;
    let msg = graph_request(
        reqwest::Method::POST,
        &base,
        &format!("/api/v1/graph/runs/{}/control", run_id),
        Some(serde_json::json!({ "action": "cancel" })),
    )?;
    serde_json::to_value(msg).map_err(|e| e.to_string())
}

fn gui_request(
    method: reqwest::Method,
    base: &str,
    endpoint: &str,
    body: Option<serde_json::Value>,
) -> Result<ServerMsg, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}{}", base, endpoint);
    let mut builder = client.request(method, &url);
    if let Some(b) = body {
        builder = builder.json(&b);
    }
    let resp = builder
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

#[tauri::command]
pub fn gui_load_config() -> Result<GuiConfigView, String> {
    load_config_view()
}

#[tauri::command]
pub fn gui_set_config(key: String, value: String) -> Result<(), String> {
    set_config_value(&key, &value)
}

// ---- 飞书一键创建应用（设备授权流）----
// 薄桥：业务在 feishu::setup；命令只负责参数转发与 tokio 阻塞等待。

fn block_on_setup<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current_thread runtime");
    rt.block_on(fut)
}

pub(crate) fn feishu_setup_begin_from(
    base: &str,
) -> Result<crate::feishu::setup::SetupBegin, String> {
    block_on_setup(crate::feishu::setup::begin(base))
}

pub(crate) fn feishu_setup_poll_from(
    base: &str,
    device_code: &str,
) -> Result<crate::feishu::setup::SetupPoll, String> {
    block_on_setup(crate::feishu::setup::poll(base, device_code))
}

#[tauri::command]
pub fn gui_feishu_setup_begin() -> Result<crate::feishu::setup::SetupBegin, String> {
    feishu_setup_begin_from(crate::feishu::setup::ACCOUNTS_BASE_URL)
}

#[tauri::command]
pub fn gui_feishu_setup_poll(
    device_code: String,
) -> Result<crate::feishu::setup::SetupPoll, String> {
    feishu_setup_poll_from(crate::feishu::setup::ACCOUNTS_BASE_URL, &device_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os(crate::paths::CONFIG_DIR_ENV);
            std::env::set_var(crate::paths::CONFIG_DIR_ENV, path);
            Self(previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref p) = self.0 {
                std::env::set_var(crate::paths::CONFIG_DIR_ENV, p);
            } else {
                std::env::remove_var(crate::paths::CONFIG_DIR_ENV);
            }
        }
    }

    #[test]
    fn commands_without_human_session_return_stable_error() {
        let tmp = TempDir::new().unwrap();
        let _guard = EnvGuard::set(tmp.path());
        // 无 session.json：各命令都应返回 session 错误而不是 panic
        let load_err = load_view("any-id").unwrap_err();
        assert!(load_err.contains("session"), "load: {}", load_err);
        let reply_err = reply_message("any-id", "hi", &[]).unwrap_err();
        assert!(reply_err.contains("session"), "reply: {}", reply_err);
        let done_err = done_message("any-id").unwrap_err();
        assert!(done_err.contains("session"), "done: {}", done_err);
        let cancel_err = cancel_message("any-id").unwrap_err();
        assert!(cancel_err.contains("session"), "cancel: {}", cancel_err);
    }

    /// 最小 mock daemon：复用 testutil 的共享实现。
    fn mock_daemon(
        responses: Vec<String>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        std::thread::JoinHandle<()>,
    ) {
        crate::testutil::mock_http_server(responses)
    }

    #[test]
    fn gui_load_config_view_from_mock_daemon() {
        let show = serde_json::json!({
            "type": "config_show_result",
            "config": { "http_port": 19527, "notify": { "default": "auto" } }
        })
        .to_string();
        let path = serde_json::json!({
            "type": "config_path",
            "path": "/tmp/agtalk-test/config.json"
        })
        .to_string();
        let (base, rx, handle) = mock_daemon(vec![show, path]);
        let view = load_config_view_from(&base).unwrap();
        assert_eq!(view.config["http_port"], 19527);
        assert_eq!(view.path, "/tmp/agtalk-test/config.json");
        assert!(rx.recv().unwrap().starts_with("GET /api/v1/config "));
        assert!(rx.recv().unwrap().starts_with("GET /api/v1/config/path "));
        handle.join().unwrap();
    }

    #[test]
    fn gui_set_config_value_to_mock_daemon() {
        let pong = serde_json::json!({ "type": "pong" }).to_string();
        let (base, rx, handle) = mock_daemon(vec![pong]);
        set_config_value_to(&base, "notify.default", "none").unwrap();
        let req = rx.recv().unwrap();
        assert!(
            req.starts_with("PATCH /api/v1/config/notify.default HTTP/1.1"),
            "{}",
            req
        );
        assert!(req.contains("\"value\":\"none\""), "{}", req);
        handle.join().unwrap();
    }

    #[test]
    fn gui_set_config_error_response_propagates_code() {
        let err = serde_json::json!({
            "type": "error",
            "code": "config_error",
            "message": "配置项不存在: no.such.key"
        })
        .to_string();
        let (base, _rx, handle) = mock_daemon(vec![err]);
        let msg = set_config_value_to(&base, "no.such.key", "x").unwrap_err();
        assert!(msg.contains("config_error"), "{}", msg);
        handle.join().unwrap();
    }

    #[test]
    fn gui_request_daemon_unreachable_has_stable_error() {
        // 端口 1 必然拒绝连接
        let err = load_config_view_from("http://127.0.0.1:1").unwrap_err();
        assert!(err.contains("daemon_unreachable"), "{}", err);
    }

    #[test]
    fn gui_feishu_setup_begin_and_poll_full_flow() {
        let begin_res = serde_json::json!({
            "device_code": "dc-1",
            "verification_uri_complete": "https://open.feishu.cn/page/launcher?user_code=ABCD-EFGH",
            "interval": 5,
            "expires_in": 600
        })
        .to_string();
        let pending = serde_json::json!({ "error": "authorization_pending" }).to_string();
        let success = serde_json::json!({
            "client_id": "cli_xxx",
            "client_secret": "sec_yyy",
            "user_info": { "open_id": "ou_zzz", "tenant_brand": "feishu" }
        })
        .to_string();
        let (base, rx, handle) = mock_daemon(vec![begin_res, pending, success]);

        let begin = feishu_setup_begin_from(&base).unwrap();
        assert_eq!(begin.device_code, "dc-1");
        assert!(begin.url.contains("user_code=ABCD-EFGH"));
        assert!(begin.url.contains("createOnly=true"));

        let poll1 = feishu_setup_poll_from(&base, &begin.device_code).unwrap();
        assert_eq!(poll1, crate::feishu::setup::SetupPoll::Pending);
        let poll2 = feishu_setup_poll_from(&base, &begin.device_code).unwrap();
        match poll2 {
            crate::feishu::setup::SetupPoll::Success {
                app_id, open_id, ..
            } => {
                assert_eq!(app_id, "cli_xxx");
                assert_eq!(open_id, "ou_zzz");
            }
            other => panic!("expected Success, got {:?}", other),
        }

        assert!(rx.recv().unwrap().contains("action=begin"));
        assert!(rx.recv().unwrap().contains("action=poll"));
        assert!(rx.recv().unwrap().contains("action=poll"));
        handle.join().unwrap();
    }
}

// ---- 图工程 SSE 订阅（M4）：Rust 侧保持长连接，事件经 Tauri emit 推给前端 ----
// 前端不直接访问 daemon（token 不下发），SSE 在进程内解析后 emit。

/// 各 run 的停止标记（前端切换 run / 卸载时置位）。
#[derive(Default)]
pub struct GraphStreamState {
    stops: std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
}

#[tauri::command]
pub fn gui_graph_stream_start(
    handle: tauri::AppHandle,
    state: tauri::State<'_, GraphStreamState>,
    run_id: String,
) -> Result<(), String> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    state
        .stops
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.clone(), stop.clone());
    std::thread::spawn(move || {
        let _ = stream_graph_events(handle, run_id, stop);
    });
    Ok(())
}

#[tauri::command]
pub fn gui_graph_stream_stop(state: tauri::State<'_, GraphStreamState>, run_id: String) {
    if let Some(stop) = state
        .stops
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&run_id)
    {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn stream_graph_events(
    handle: tauri::AppHandle,
    run_id: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    use std::io::BufRead;
    let base = gui_base_url()?;
    let session =
        crate::identity::human_session::load().map_err(|e| format!("human session 不可用: {e}"))?;
    let url = format!("{}/api/v1/graph/events/stream?run_id={}", base, run_id);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("X-AgTalk-Human-Token", session.token)
        .send()
        .map_err(|e| format!("daemon_unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("http_error: HTTP {}", resp.status()));
    }
    let reader = std::io::BufReader::new(resp);
    let mut event_id = String::new();
    let mut data = String::new();
    for line in reader.lines() {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let l = line.trim();
        if l.is_empty() {
            if !data.is_empty() {
                let _ = handle.emit(
                    "graph-event",
                    serde_json::json!({ "run_id": run_id, "id": event_id, "data": data }),
                );
            }
            event_id.clear();
            data.clear();
            continue;
        }
        if let Some(v) = l.strip_prefix("id:") {
            event_id = v.trim().to_string();
        } else if let Some(v) = l.strip_prefix("data:") {
            data = v.trim().to_string();
        }
    }
    Ok(())
}
