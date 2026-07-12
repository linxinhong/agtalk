//! HTTP server 共享状态。

use crate::config::AgConfig;
use crate::human::popup::PopupTransport;
use crate::notify::NotifyLimiter;
use crate::storage::Storage;
use crate::transport::wake::SubscriberRegistry;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    pub config: Arc<AgConfig>,
    pub registry: SubscriberRegistry,
    pub dot_agtalk: PathBuf,
    pub notify_limiter: Arc<NotifyLimiter>,
    /// 桌面弹窗投递：daemon 启用，测试默认 disabled（不拉起真实子进程）。
    pub popup: Arc<PopupTransport>,
}

impl AppState {
    pub fn new(storage: Storage, config: AgConfig, dot_agtalk: PathBuf) -> Self {
        Self {
            storage,
            config: Arc::new(config),
            registry: SubscriberRegistry::new(),
            dot_agtalk,
            notify_limiter: Arc::new(NotifyLimiter::default_cooldown()),
            popup: Arc::new(PopupTransport::disabled()),
        }
    }
}
