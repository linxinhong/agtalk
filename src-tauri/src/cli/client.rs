//! daemon IPC 客户端：连接 Unix socket，发送 ClientMsg，接收 ServerMsg。

use crate::ipc::{ClientMsg, ServerMsg};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// 生成连接失败时的诊断信息（proxy / stale socket 建议）。
fn connection_diagnostic(socket_path: &str) -> String {
    let mut lines = vec![
        format!("socket 路径: {}", socket_path),
        "agtalk 本地 IPC 使用 Unix domain socket，不应经过网络代理。".to_string(),
    ];
    let proxy_vars = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"];
    let has_proxy = proxy_vars.iter().any(|n| std::env::var(n).is_ok());
    if has_proxy {
        lines.push("检测到网络代理环境变量；如本地连接被拦截，请检查代理工具是否覆盖/隔离了 HOME / XDG / AGTALK_CONFIG_DIR。".to_string());
    }
    lines.push("可尝试执行 `agtalk daemon restart` 修复 stale 的 pid/socket 文件。".to_string());
    lines.join("\n")
}

pub struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    #[allow(dead_code)]
    session_id: Option<String>,
    #[allow(dead_code)]
    token: Option<String>,
}

impl Client {
    pub async fn connect(socket_path: &str) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await.map_err(|e| {
            anyhow::anyhow!(
                "无法连接 daemon: {}\n{}",
                e,
                connection_diagnostic(socket_path)
            )
        })?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
            session_id: None,
            token: None,
        })
    }

    pub async fn connect_and_auth(
        socket_path: &str,
        session_id: &str,
        token: &str,
    ) -> Result<Self> {
        let mut client = Self::connect(socket_path).await?;
        let resp = client.auth(session_id, token).await?;
        match resp {
            ServerMsg::Ok { .. } => {
                client.session_id = Some(session_id.to_string());
                client.token = Some(token.to_string());
                Ok(client)
            }
            ServerMsg::Error { code, message } => {
                anyhow::bail!("认证失败 [{}]: {}", code, message)
            }
            _ => anyhow::bail!("认证返回异常"),
        }
    }

    async fn request(&mut self, msg: &ClientMsg) -> Result<ServerMsg> {
        let json = crate::ipc::serialize(msg);
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        crate::ipc::deserialize::<ServerMsg>(&line).context("无法解析 daemon 响应")
    }

    pub async fn auth(&mut self, session_id: &str, token: &str) -> Result<ServerMsg> {
        self.request(&ClientMsg::Auth {
            session_id: session_id.to_string(),
            token: token.to_string(),
        })
        .await
    }

    pub async fn ping(&mut self) -> Result<ServerMsg> {
        self.request(&ClientMsg::Ping).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn join(
        &mut self,
        workspace_root: &str,
        workspace_name: &str,
        name: &str,
        role: &str,
        intro: &str,
        transport: &str,
        notify_config: serde_json::Value,
        runtime_config: serde_json::Value,
        takeover: bool,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Join {
            workspace_root: workspace_root.to_string(),
            workspace_name: workspace_name.to_string(),
            name: name.to_string(),
            role: role.to_string(),
            intro: intro.to_string(),
            transport: transport.to_string(),
            notify_config,
            runtime_config,
            capabilities: vec![],
            takeover,
        })
        .await
    }

    pub async fn leave(&mut self, session_id: Option<&str>) -> Result<ServerMsg> {
        self.request(&ClientMsg::Leave {
            session_id: session_id.map(|s| s.to_string()),
        })
        .await
    }

    pub async fn cleanup(&mut self, dry_run: bool) -> Result<ServerMsg> {
        self.request(&ClientMsg::Cleanup { dry_run }).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send(
        &mut self,
        to: &str,
        body: &str,
        conversation_id: Option<&str>,
        reply_to: Option<&str>,
        correlation_id: Option<&str>,
        content_type: Option<&str>,
        metadata: Option<serde_json::Value>,
        notify: bool,
        send_enter: Option<bool>,
        attachments: Vec<crate::ipc::SendAttachment>,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Send {
            sender: None,
            to: to.to_string(),
            body: body.to_string(),
            conversation_id: conversation_id.map(|s| s.to_string()),
            reply_to: reply_to.map(|s| s.to_string()),
            correlation_id: correlation_id.map(|s| s.to_string()),
            content_type: content_type.unwrap_or("text").to_string(),
            metadata,
            notify,
            send_enter,
            attachments,
        })
        .await
    }

    pub async fn inbox(
        &mut self,
        participant: &str,
        status: Option<&str>,
        limit: u32,
        peek: bool,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Inbox {
            sender: None,
            participant: participant.to_string(),
            status: status.map(|s| s.to_string()),
            limit,
            peek,
        })
        .await
    }

    pub async fn detail(&mut self, msg_id: &str) -> Result<ServerMsg> {
        self.request(&ClientMsg::Detail {
            msg_id: msg_id.to_string(),
            participant: None,
        })
        .await
    }

    pub async fn attachment(&mut self, attachment_id: &str) -> Result<ServerMsg> {
        self.request(&ClientMsg::Attachment {
            attachment_id: attachment_id.to_string(),
            participant: None,
        })
        .await
    }

    pub async fn done(
        &mut self,
        msg_id: &str,
        participant: &str,
        attachments: Vec<crate::ipc::SendAttachment>,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Done {
            sender: None,
            msg_id: msg_id.to_string(),
            participant: participant.to_string(),
            attachments,
        })
        .await
    }

    pub async fn list_participants(&mut self, participant_type: Option<&str>) -> Result<ServerMsg> {
        self.request(&ClientMsg::ListParticipants {
            participant_type: participant_type.map(|s| s.to_string()),
        })
        .await
    }

    pub async fn list_conversations(&mut self, participant: Option<&str>) -> Result<ServerMsg> {
        self.request(&ClientMsg::ListConversations {
            participant: participant.map(|s| s.to_string()),
        })
        .await
    }

    #[allow(dead_code)]
    pub async fn get_messages(
        &mut self,
        conversation_id: &str,
        limit: u32,
        before: Option<&str>,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::GetMessages {
            conversation_id: conversation_id.to_string(),
            limit,
            before: before.map(|s| s.to_string()),
            participant: None,
        })
        .await
    }

    pub async fn whoami(&mut self) -> Result<ServerMsg> {
        self.request(&ClientMsg::WhoAmI).await
    }

    // ── Ask / Reply ────────────────────────────
    pub async fn ask(
        &mut self,
        to: &str,
        body: &str,
        choices: &[String],
        timeout_secs: u64,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Ask {
            sender: None,
            to: to.to_string(),
            body: body.to_string(),
            choices: choices.to_vec(),
            timeout_secs,
        })
        .await
    }

    #[allow(dead_code)]
    pub async fn reply(&mut self, msg_id: &str, choice: &str, reason: &str) -> Result<ServerMsg> {
        self.request(&ClientMsg::Reply {
            sender: None,
            msg_id: msg_id.to_string(),
            choice: choice.to_string(),
            reason: reason.to_string(),
        })
        .await
    }

    pub async fn wait(&mut self, msg_id: &str, timeout_secs: u64) -> Result<ServerMsg> {
        self.request(&ClientMsg::Wait {
            sender: None,
            msg_id: msg_id.to_string(),
            timeout_secs,
        })
        .await
    }
}
