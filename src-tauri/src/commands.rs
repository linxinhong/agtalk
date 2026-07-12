//! Tauri 命令桥（薄）。
//! 每个 #[tauri::command] 只做：接收前端参数 → 调对应领域函数 → 返回结果。
//! popup 窗口的业务全部经 HumanClient（human/client.rs）走 daemon API，
//! token 留在 Rust 侧，不下发到前端。

use crate::human::client::HumanClient;
use crate::human::popup::POPUP_SURFACE;
use crate::proto::ServerMsg;
use crate::routing::Message;
use serde::Serialize;

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
    choice: Option<&str>,
) -> Result<String, String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .reply(POPUP_SURFACE, message_id, body, choice)
        .map_err(|e| e.to_string())
}

/// 标记消息完成。
pub(crate) fn done_message(message_id: &str) -> Result<(), String> {
    let client = HumanClient::from_local().map_err(|e| e.to_string())?;
    client
        .done(POPUP_SURFACE, message_id)
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
    choice: Option<String>,
) -> Result<String, String> {
    reply_message(&state.message_id, &body, choice.as_deref())
}

#[tauri::command]
pub fn popup_done(state: tauri::State<'_, PopupState>) -> Result<(), String> {
    done_message(&state.message_id)
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
        // 无 session.json：三个命令都应返回 session 错误而不是 panic
        let load_err = load_view("any-id").unwrap_err();
        assert!(load_err.contains("session"), "load: {}", load_err);
        let reply_err = reply_message("any-id", "hi", None).unwrap_err();
        assert!(reply_err.contains("session"), "reply: {}", reply_err);
        let done_err = done_message("any-id").unwrap_err();
        assert!(done_err.contains("session"), "done: {}", done_err);
    }

    /// 最小 mock daemon：按序返回预置响应，请求行经 channel 回报给测试断言。
    fn mock_daemon(
        responses: Vec<String>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                tx.send(request_line.trim_end().to_string()).unwrap();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end();
                    if let Some(v) = trimmed
                        .to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                    {
                        content_length = v.trim().parse().unwrap();
                    }
                    if trimmed.is_empty() {
                        break;
                    }
                }
                if content_length > 0 {
                    let mut buf = vec![0u8; content_length];
                    reader.read_exact(&mut buf).unwrap();
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });
        (format!("http://127.0.0.1:{}", port), rx, handle)
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
        assert_eq!(
            rx.recv().unwrap(),
            "PATCH /api/v1/config/notify.default HTTP/1.1"
        );
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
}
