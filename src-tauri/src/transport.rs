//! 传输抽象层：消息投递到不同终端/渠道。

use anyhow::Result;
use std::sync::Arc;

/// 传输 trait：每种传输方式（终端、弹窗、IM 等）实现此 trait
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// 传输类型标识
    fn kind(&self) -> &str;

    /// 投递消息到目标终端/渠道
    async fn deliver(
        &self,
        msg_id: &str,
        from: &str,
        body: &str,
        transport_config: &str,
    ) -> Result<()>;

    /// 发送提醒通知（非消息正文）
    #[allow(dead_code)]
    async fn notify(
        &self,
        msg_id: &str,
        from: &str,
        to: &str,
        transport_config: &str,
    ) -> Result<()>;
}

/// 传输注册表：持有所有已初始化的传输实例
pub struct TransportRegistry {
    transports: Vec<Arc<dyn Transport>>,
}

impl Default for TransportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportRegistry {
    pub fn new() -> Self {
        Self {
            transports: Vec::new(),
        }
    }

    pub fn register(&mut self, transport: Arc<dyn Transport>) {
        self.transports.push(transport);
    }

    pub fn get(&self, kind: &str) -> Option<Arc<dyn Transport>> {
        self.transports.iter().find(|t| t.kind() == kind).cloned()
    }
}

/// 终端传输：通过 Zellij/Tmux write-chars 投递消息到目标 pane
pub struct TerminalTransport;

impl Default for TerminalTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalTransport {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Transport for TerminalTransport {
    fn kind(&self) -> &str {
        "terminal"
    }

    async fn deliver(
        &self,
        msg_id: &str,
        from: &str,
        body: &str,
        _transport_config: &str,
    ) -> Result<()> {
        // TODO: 调用 Zellij/Tmux write-chars
        // 当前为 stub，后续集成真正的终端多路复用器操作
        tracing::info!(
            "[terminal] deliver msg={} from={} body_len={}",
            &msg_id[..8.min(msg_id.len())],
            from,
            body.len()
        );
        Ok(())
    }

    #[allow(dead_code)]
    async fn notify(
        &self,
        msg_id: &str,
        from: &str,
        to: &str,
        _transport_config: &str,
    ) -> Result<()> {
        tracing::info!(
            "[terminal] notify msg={} from={} to={}",
            &msg_id[..8.min(msg_id.len())],
            from,
            to
        );
        Ok(())
    }
}

/// 弹窗传输：触发桌面弹窗通知人类用户
pub struct PopupTransport;

impl Default for PopupTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl PopupTransport {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Transport for PopupTransport {
    fn kind(&self) -> &str {
        "popup"
    }

    async fn deliver(
        &self,
        msg_id: &str,
        from: &str,
        body: &str,
        _transport_config: &str,
    ) -> Result<()> {
        // 启动独立审批弹窗进程（agtalk __popup <msg_id>）
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("agtalk"));
        match std::process::Command::new(&exe)
            .arg("__popup")
            .arg(msg_id)
            .spawn()
        {
            Ok(child) => tracing::info!(
                "[popup] 已启动审批窗口 msg={} from={} pid={}",
                &msg_id[..8.min(msg_id.len())],
                from,
                child.id()
            ),
            Err(e) => tracing::error!(
                "[popup] 启动审批窗口失败 msg={} from={}: {}",
                &msg_id[..8.min(msg_id.len())],
                from,
                e
            ),
        }
        let _ = body;
        Ok(())
    }

    #[allow(dead_code)]
    async fn notify(
        &self,
        msg_id: &str,
        from: &str,
        to: &str,
        _transport_config: &str,
    ) -> Result<()> {
        tracing::info!(
            "[popup] notify msg={} from={} to={}",
            &msg_id[..8.min(msg_id.len())],
            from,
            to
        );
        Ok(())
    }
}
