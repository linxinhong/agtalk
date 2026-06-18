//! daemon IPC 客户端：连接 Unix socket，发送 ClientMsg，接收 ServerMsg。

use crate::ipc::{ClientMsg, ServerMsg};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    pub async fn connect(socket_path: &str) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .with_context(|| format!("无法连接 daemon: {}", socket_path))?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(reader),
            writer,
        })
    }

    async fn request(&mut self, msg: &ClientMsg) -> Result<ServerMsg> {
        let json = crate::ipc::serialize(msg);
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;

        let mut line = String::new();
        self.reader.read_line(&mut line).await?;
        crate::ipc::deserialize::<ServerMsg>(&line)
            .context("无法解析 daemon 响应")
    }

    pub async fn ping(&mut self) -> Result<ServerMsg> {
        self.request(&ClientMsg::Ping).await
    }

    pub async fn send(
        &mut self,
        to: &str,
        body: &str,
        conversation_id: Option<&str>,
        reply_to: Option<&str>,
        correlation_id: Option<&str>,
        content_type: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) -> Result<ServerMsg> {
        let sender = std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "anonymous".into());
        self.request(&ClientMsg::Send {
            sender,
            to: to.to_string(),
            body: body.to_string(),
            conversation_id: conversation_id.map(|s| s.to_string()),
            reply_to: reply_to.map(|s| s.to_string()),
            correlation_id: correlation_id.map(|s| s.to_string()),
            content_type: content_type.unwrap_or("text").to_string(),
            metadata,
        }).await
    }

    pub async fn inbox(
        &mut self,
        participant: &str,
        status: Option<&str>,
        limit: u32,
    ) -> Result<ServerMsg> {
        let sender = std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "anonymous".into());
        self.request(&ClientMsg::Inbox {
            sender,
            participant: participant.to_string(),
            status: status.map(|s| s.to_string()),
            limit,
        }).await
    }

    pub async fn done(&mut self, msg_id: &str, participant: &str) -> Result<ServerMsg> {
        let sender = std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "anonymous".into());
        self.request(&ClientMsg::Done {
            sender,
            msg_id: msg_id.to_string(),
            participant: participant.to_string(),
        }).await
    }

    pub async fn register(
        &mut self,
        name: &str,
        participant_type: &str,
        display_name: &str,
        transport: &str,
        transport_config: &str,
    ) -> Result<ServerMsg> {
        self.request(&ClientMsg::Register {
            name: name.to_string(),
            participant_type: participant_type.to_string(),
            display_name: display_name.to_string(),
            transport: transport.to_string(),
            transport_config: serde_json::from_str(transport_config).unwrap_or_default(),
        }).await
    }

    pub async fn unregister(&mut self, name: &str) -> Result<ServerMsg> {
        self.request(&ClientMsg::Unregister {
            name: name.to_string(),
        }).await
    }

    pub async fn list_participants(&mut self, participant_type: Option<&str>) -> Result<ServerMsg> {
        self.request(&ClientMsg::ListParticipants {
            participant_type: participant_type.map(|s| s.to_string()),
        }).await
    }

    pub async fn list_conversations(&mut self, participant: Option<&str>) -> Result<ServerMsg> {
        self.request(&ClientMsg::ListConversations {
            participant: participant.map(|s| s.to_string()),
        }).await
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
        }).await
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
        let sender = std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "anonymous".into());
        self.request(&ClientMsg::Ask {
            sender,
            to: to.to_string(),
            body: body.to_string(),
            choices: choices.to_vec(),
            timeout_secs,
        }).await
    }

    #[allow(dead_code)]
    pub async fn reply(
        &mut self,
        msg_id: &str,
        choice: &str,
        reason: &str,
    ) -> Result<ServerMsg> {
        let sender = std::env::var("AGTALK_AGENT_NAME").unwrap_or_else(|_| "anonymous".into());
        self.request(&ClientMsg::Reply {
            sender,
            msg_id: msg_id.to_string(),
            choice: choice.to_string(),
            reason: reason.to_string(),
        }).await
    }

}
