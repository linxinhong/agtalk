//! 订阅者注册表：按 address 维护 broadcast channel，send 后唤醒 SSE 连接。

use crate::routing::Message;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub message: Message,
}

#[derive(Clone)]
pub struct SubscriberRegistry {
    senders: Arc<Mutex<HashMap<String, broadcast::Sender<SseEvent>>>>,
}

impl Default for SubscriberRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriberRegistry {
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self, address: &str) -> broadcast::Receiver<SseEvent> {
        let mut senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        let sender = senders
            .entry(address.to_string())
            .or_insert_with(|| broadcast::channel(128).0);
        sender.subscribe()
    }

    pub fn notify(&self, address: &str, event: SseEvent) {
        let senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = senders.get(address) {
            let _ = sender.send(event);
        }
    }
}
