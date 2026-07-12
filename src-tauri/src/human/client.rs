//! HumanClient：本机 human 客户端（popup/GUI）的 daemon API + SSE 客户端。
//!
//! 认证用 `<config_dir>/human/session.json` 的 token（`X-AgTalk-Human-Token`），
//! 只访问 `/api/v1/human/*` 与 `GET /api/v1/events`。token 留在 Rust 侧，
//! 不下发到 Tauri 前端（frontend 经 commands.rs 薄桥调用本模块）。
//! 协议见 docs/human-surfaces.md。

use crate::config::AgConfig;
use crate::identity::human_session::{self, HumanSession};
use crate::proto::{LookupMailbox, ServerMsg};
use crate::routing::Message;
use std::io::{BufRead, BufReader, Read};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HumanClientError {
    #[error("配置错误: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("human session 不可用（请重启 daemon 重新颁发）: {0}")]
    Session(#[from] crate::identity::IdentityError),
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("daemon 返回错误: {code}（{message}）")]
    Daemon { code: String, message: String },
    #[error("响应格式错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("响应类型不符: {0}")]
    UnexpectedResponse(String),
}

/// human API 客户端。blocking 实现：popup 进程与 Tauri 命令都是短调用，
/// 不引入 async 运行时依赖。
pub struct HumanClient {
    base_url: String,
    session: HumanSession,
    http: reqwest::blocking::Client,
}

impl HumanClient {
    /// 从本机配置与 human session 构造（popup/GUI 的标准入口）。
    pub fn from_local() -> Result<Self, HumanClientError> {
        let cfg = AgConfig::load()?;
        let session = human_session::load()?;
        Ok(Self::new(
            format!("http://127.0.0.1:{}", cfg.http_port),
            session,
        ))
    }

    pub fn new(base_url: impl Into<String>, session: HumanSession) -> Self {
        Self {
            base_url: base_url.into(),
            session,
            http: reqwest::blocking::Client::new(),
        }
    }

    /// human mailbox 地址（消息归属与展示用）。
    pub fn address(&self) -> &str {
        &self.session.address
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<ServerMsg, HumanClientError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("X-AgTalk-Human-Token", &self.session.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send()?;
        let msg: ServerMsg = resp.json()?;
        if let ServerMsg::Error { code, message } = msg {
            return Err(HumanClientError::Daemon { code, message });
        }
        Ok(msg)
    }

    /// human 收件箱（all=true 含已 done）。
    pub fn inbox(&self, all: bool) -> Result<Vec<Message>, HumanClientError> {
        match self.request(
            reqwest::Method::GET,
            &format!("/api/v1/human/inbox?all={}", all),
            None,
        )? {
            ServerMsg::InboxResult { messages } => Ok(messages),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 读指定消息（标 read）并返回详情。
    pub fn read(&self, message_id: &str) -> Result<Message, HumanClientError> {
        match self.request(
            reqwest::Method::POST,
            "/api/v1/human/read",
            Some(serde_json::json!({ "message_id": message_id })),
        )? {
            ServerMsg::MsgDetail(m) => Ok(m),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 回复消息；`choices` 为审批选中项（可多选，空切片 = 纯文本回复）。
    /// 成功返回回复消息 id。审批已被其他 surface 处理时返回 Daemon{code: "already_resolved"}。
    pub fn reply(
        &self,
        surface: &str,
        message_id: &str,
        body: &str,
        choices: &[String],
    ) -> Result<String, HumanClientError> {
        let payload = serde_json::json!({
            "message_id": message_id,
            "body": body,
            "choices": choices,
            "surface": surface,
        });
        match self.request(reqwest::Method::POST, "/api/v1/human/reply", Some(payload))? {
            ServerMsg::Ok { id } => Ok(id),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 标记消息完成。
    pub fn done(&self, surface: &str, message_id: &str) -> Result<(), HumanClientError> {
        match self.request(
            reqwest::Method::POST,
            "/api/v1/human/done",
            Some(serde_json::json!({
                "message_id": message_id,
                "surface": surface,
            })),
        )? {
            ServerMsg::Ok { .. } => Ok(()),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 取消消息：给原发送方回「（已取消）」并终结原消息。成功返回取消通知消息 id。
    pub fn cancel(&self, surface: &str, message_id: &str) -> Result<String, HumanClientError> {
        match self.request(
            reqwest::Method::POST,
            "/api/v1/human/cancel",
            Some(serde_json::json!({
                "message_id": message_id,
                "surface": surface,
            })),
        )? {
            ServerMsg::Ok { id } => Ok(id),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// delivery 回执：surface 确认已展示该消息。
    pub fn delivery_ack(&self, surface: &str, message_id: &str) -> Result<(), HumanClientError> {
        match self.request(
            reqwest::Method::POST,
            "/api/v1/human/delivery/ack",
            Some(serde_json::json!({
                "message_id": message_id,
                "surface": surface,
            })),
        )? {
            ServerMsg::Ok { .. } => Ok(()),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 在线 agent 列表（主动发信用）。
    pub fn agents(&self) -> Result<Vec<LookupMailbox>, HumanClientError> {
        match self.request(reqwest::Method::GET, "/api/v1/human/agents", None)? {
            ServerMsg::LookupResult { mailboxes } => Ok(mailboxes),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// human 主动发信（to 必须是 agents 列表中的 UUID）。
    pub fn send(
        &self,
        surface: &str,
        to: &str,
        body: &str,
        subject: Option<&str>,
    ) -> Result<String, HumanClientError> {
        let mut payload = serde_json::json!({
            "to": to,
            "body": body,
            "surface": surface,
        });
        if let Some(s) = subject {
            payload["subject"] = serde_json::Value::String(s.to_string());
        }
        match self.request(reqwest::Method::POST, "/api/v1/human/send", Some(payload))? {
            ServerMsg::Ok { id } => Ok(id),
            other => Err(HumanClientError::UnexpectedResponse(format!("{:?}", other))),
        }
    }

    /// 订阅 human mailbox 的 SSE 事件流（blocking 迭代器）。
    /// `last_event_id` 为 Some 时从该 event 之后重放；None 只收新事件。
    pub fn events(&self, last_event_id: Option<i64>) -> Result<EventStream, HumanClientError> {
        let mut req = self
            .http
            .get(format!("{}/api/v1/events", self.base_url))
            .header("X-AgTalk-Human-Token", &self.session.token);
        if let Some(id) = last_event_id {
            req = req.header("Last-Event-ID", id.to_string());
        }
        let resp = req.send()?;
        Ok(EventStream::new(Box::new(resp)))
    }
}

/// SSE 事件流：逐条解析 `event: message` 的 data（Message JSON）。
/// 迭代器在连接关闭（EOF）时结束；解析失败跳过该行继续。
pub struct EventStream {
    reader: BufReader<Box<dyn Read + Send>>,
}

impl EventStream {
    fn new(read: Box<dyn Read + Send>) -> Self {
        Self {
            reader: BufReader::new(read),
        }
    }
}

impl Iterator for EventStream {
    type Item = Result<Message, HumanClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut data = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None, // EOF：连接关闭
                Ok(_) => {}
                Err(e) => return Some(Err(HumanClientError::Io(e))),
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                // 事件边界：有 data 才产出（跳过 keep-alive 注释块）
                if data.is_empty() {
                    continue;
                }
                let parsed = serde_json::from_str::<Message>(&data).map_err(HumanClientError::Json);
                return Some(parsed);
            }
            if trimmed.starts_with(':') {
                continue; // SSE 注释（keep-alive）
            }
            if let Some(value) = trimmed.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.trim_start());
            }
            // id:/event: 行目前不需要（Message 自带 event_id）
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::mailbox::{create, ensure_human};
    use crate::routing::{send::send, SendRequest};
    use crate::server::http::routes;
    use crate::server::state::AppState;
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

    struct Fixture {
        base_url: String,
        session: HumanSession,
        storage: crate::storage::Storage,
        agent_addr: String,
        _tmp: TempDir,
        _guard: EnvGuard,
    }

    /// 在 ephemeral 端口起真实 HTTP server，HumanClient 走真实请求。
    async fn start_server() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let guard = EnvGuard::set(tmp.path());
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        let cfg = AgConfig::default();
        let human_addr = ensure_human(&storage, &cfg.human).unwrap();
        let session = human_session::ensure(&human_addr, &cfg.human.name).unwrap();
        let agent_addr = create(&storage, "nora", "", "").unwrap();
        let state = AppState::new(storage.clone(), cfg, tmp.path().join(".agtalk"));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, routes(state)).await.unwrap();
        });

        Fixture {
            base_url: format!("http://{}", addr),
            session,
            storage,
            agent_addr,
            _tmp: tmp,
            _guard: guard,
        }
    }

    fn send_to_human(fx: &Fixture, body: &str, content_type: &str, metadata: &str) -> Message {
        send(
            &fx.storage,
            SendRequest {
                to: &fx.session.address,
                to_name: "human",
                from: &fx.agent_addr,
                from_name: "nora",
                body,
                content_type,
                reply_to_id: None,
                subject: None,
                metadata,
                more_coming: false,
            },
        )
        .unwrap()
    }

    /// 在独立线程执行阻塞客户端调用：reqwest blocking Client 的构造/析构
    /// 不能在 tokio runtime 上下文里（内部 runtime drop 会 panic）。
    fn run_blocking<F>(base_url: String, session: HumanSession, f: F)
    where
        F: FnOnce(HumanClient) + Send + 'static,
    {
        std::thread::spawn(move || f(HumanClient::new(base_url, session)))
            .join()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_inbox_read_reply_done_roundtrip() {
        let fx = start_server().await;
        let msg = send_to_human(&fx, "deploy?", "text", "{}");

        let msg_id = msg.id.clone();
        run_blocking(fx.base_url.clone(), fx.session.clone(), move |c| {
            let inbox = c.inbox(false).unwrap();
            assert_eq!(inbox.len(), 1);

            let detail = c.read(&msg_id[..8]).unwrap();
            assert_eq!(detail.id, msg_id);

            let reply_id = c.reply("popup", &msg_id, "go", &[]).unwrap();
            assert!(!reply_id.is_empty());

            c.done("popup", &msg_id).unwrap();
        });

        let m = crate::routing::lookup::detail(&fx.storage, &msg.id)
            .unwrap()
            .unwrap();
        assert_eq!(m.status, "done");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_cancel_returns_cancelled_reply() {
        let fx = start_server().await;
        let msg = send_to_human(&fx, "deploy?", "text", "{}");

        let msg_id = msg.id.clone();
        run_blocking(fx.base_url.clone(), fx.session.clone(), move |c| {
            let cancel_id = c.cancel("popup", &msg_id).unwrap();
            assert!(!cancel_id.is_empty());
        });

        let m = crate::routing::lookup::detail(&fx.storage, &msg.id)
            .unwrap()
            .unwrap();
        assert_eq!(m.status, "done");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_reply_already_resolved_surfaces_error_code() {
        let fx = start_server().await;
        let msg = send_to_human(
            &fx,
            "approve?",
            "approval_request",
            r#"{"choices":["yes","no"],"select_only":true}"#,
        );

        let id = msg.id.clone();
        run_blocking(fx.base_url.clone(), fx.session.clone(), move |c| {
            c.reply("popup", &id, "", &["yes".to_string()]).unwrap();
            match c.reply("gui", &id, "", &["no".to_string()]) {
                Err(HumanClientError::Daemon { code, message }) => {
                    assert_eq!(code, "already_resolved");
                    assert!(!message.is_empty());
                }
                other => panic!("expected already_resolved, got {:?}", other.is_ok()),
            }
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_agents_and_send() {
        let fx = start_server().await;
        let agent_addr = fx.agent_addr.clone();
        run_blocking(fx.base_url.clone(), fx.session.clone(), move |c| {
            let agents = c.agents().unwrap();
            assert_eq!(agents.len(), 1);
            assert_eq!(agents[0].address, agent_addr);

            let id = c
                .send("popup", &agent_addr, "hello nora", Some("greet"))
                .unwrap();
            assert!(!id.is_empty());
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_wrong_token_rejected() {
        let fx = start_server().await;
        let mut bad_session = fx.session.clone();
        bad_session.token = "wrong".to_string();
        run_blocking(fx.base_url.clone(), bad_session, |c| match c.inbox(false) {
            Err(HumanClientError::Daemon { code, .. }) => assert_eq!(code, "auth_failed"),
            other => panic!("expected auth_failed, got {:?}", other.is_ok()),
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_events_replays_message_after_last_event_id() {
        let fx = start_server().await;
        let msg = send_to_human(&fx, "via sse", "text", "{}");

        let msg_id = msg.id.clone();
        run_blocking(fx.base_url.clone(), fx.session.clone(), move |c| {
            let mut stream = c.events(Some(0)).unwrap();
            let first = stream
                .next()
                .expect("replay should yield the message")
                .unwrap();
            assert_eq!(first.id, msg_id);
            assert_eq!(first.body, "via sse");
        });
    }
}
