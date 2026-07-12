//! PopupTransport：对每条 human delivery 拉起 `agtalk __popup <message-id>` 桌面弹窗。
//!
//! 设计见 docs/design.md §3.6 第二阶段。弹窗进程是薄客户端（human token + API），
//! daemon 只负责拉起与监控：子进程退出即释放 in-flight 名额，"关闭且无回复 = dismissed"
//! 只记日志不改状态（消息仍在 human inbox，可经其它 surface/再次拉起处理）。
//! spawn 失败标 delivery failed（attempts+1 记 error，可被重试捞取）。

use super::delivery;
use crate::routing::Message;
use crate::storage::Storage;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// 弹窗 surface 名（与 config.human.surfaces 中的值一致）。
pub const POPUP_SURFACE: &str = "popup";

pub struct PopupTransport {
    enabled: bool,
    /// 已有弹窗进程在运行的 message id，防止同一消息重复弹窗。
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl PopupTransport {
    /// 未启用：dispatch 全部 no-op（测试与非桌面环境默认）。
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn enabled() -> Self {
        Self {
            enabled: true,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 为一条发往 human 的消息拉起弹窗。
    /// surface 列表不含 popup、未启用或该消息已有弹窗在飞行中时跳过。
    pub fn dispatch(&self, storage: &Storage, msg: &Message, surfaces: &[String]) {
        if !self.claim(&msg.id, surfaces) {
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.release(&msg.id);
                tracing::warn!("popup spawn skipped: current_exe failed: {}", e);
                self.mark_failed(storage, msg, &e.to_string());
                return;
            }
        };
        match std::process::Command::new(exe)
            .arg("__popup")
            .arg(&msg.id)
            .spawn()
        {
            Ok(mut child) => {
                let in_flight = Arc::clone(&self.in_flight);
                let message_id = msg.id.clone();
                // ChildMonitor：等子进程退出后释放名额；无回复关闭即 dismissed，仅日志
                std::thread::spawn(move || {
                    match child.wait() {
                        Ok(status) => tracing::info!(
                            message_id = %message_id,
                            exit = %status,
                            "popup window closed"
                        ),
                        Err(e) => tracing::warn!(
                            message_id = %message_id,
                            "popup monitor failed: {}", e
                        ),
                    }
                    in_flight
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&message_id);
                });
            }
            Err(e) => {
                self.release(&msg.id);
                tracing::warn!("popup spawn failed: {}", e);
                self.mark_failed(storage, msg, &e.to_string());
            }
        }
    }

    /// 占名额：满足启用 + surface 含 popup + 该消息无在途弹窗时插入并返回 true。
    fn claim(&self, message_id: &str, surfaces: &[String]) -> bool {
        if !self.enabled || !surfaces.iter().any(|s| s == POPUP_SURFACE) {
            return false;
        }
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(message_id.to_string())
    }

    fn release(&self, message_id: &str) {
        self.in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(message_id);
    }

    fn mark_failed(&self, storage: &Storage, msg: &Message, error: &str) {
        let error = error.to_string();
        if let Err(e) = delivery::deliver_via(storage, msg, POPUP_SURFACE, |_| Err(error)) {
            tracing::warn!("popup delivery mark failed: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces_with_popup() -> Vec<String> {
        vec!["popup".to_string(), "feishu".to_string()]
    }

    #[test]
    fn disabled_transport_never_claims() {
        let t = PopupTransport::disabled();
        assert!(!t.claim("m1", &surfaces_with_popup()));
    }

    #[test]
    fn surface_without_popup_never_claims() {
        let t = PopupTransport::enabled();
        assert!(!t.claim("m1", &["feishu".to_string()]));
    }

    #[test]
    fn claim_dedups_in_flight_message() {
        let t = PopupTransport::enabled();
        assert!(t.claim("m1", &surfaces_with_popup()));
        // 同一消息已有弹窗在途：不重复弹
        assert!(!t.claim("m1", &surfaces_with_popup()));
        // 不同消息正常弹
        assert!(t.claim("m2", &surfaces_with_popup()));
        // 弹窗关闭（release）后同消息可再次弹
        t.release("m1");
        assert!(t.claim("m1", &surfaces_with_popup()));
    }
}
