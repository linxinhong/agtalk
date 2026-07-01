//! HTTP server 共享状态。

use crate::config::AgConfig;
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
}

impl AppState {
    pub fn new(storage: Storage, config: AgConfig, dot_agtalk: PathBuf) -> Self {
        Self {
            storage,
            config: Arc::new(config),
            registry: SubscriberRegistry::new(),
            dot_agtalk,
        }
    }
}
