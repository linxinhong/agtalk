//! PopupTransport：对每条 human delivery 拉起 `agtalk __popup <message-id>` 桌面弹窗。
//!
//! 设计见 docs/design.md §3.6 第二阶段。弹窗进程是薄客户端（human token + API），
//! daemon 只负责拉起与监控：子进程退出即释放 in-flight 名额，"关闭且无回复 = dismissed"
//! 只记日志不改状态（消息仍在 human inbox，可经其它 surface/再次拉起处理）。
//! spawn 失败标 delivery failed（attempts+1 记 error，可被重试捞取）。

use super::delivery;
use crate::routing::Message;
use crate::storage::Storage;
use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 弹窗 surface 名（与 config.human.surfaces 中的值一致）。
pub const POPUP_SURFACE: &str = "popup";

/// ChildMonitor 轮询间隔：try_wait 短轮询代替阻塞 wait，
/// 让 settle（他端抢答关闭弹窗）能拿到 child 锁。
const MONITOR_POLL: Duration = Duration::from_millis(200);

/// 在途弹窗子进程占位槽：spawn 前占位防重复弹窗，spawn 成功后填入句柄供 settle 关闭。
type ChildSlot = Arc<Mutex<Option<Child>>>;

pub struct PopupTransport {
    enabled: bool,
    /// 在途弹窗：message id → 子进程占位槽。防同一消息重复弹窗，
    /// 并支持 settle（他端抢答后关闭本端弹窗）。
    in_flight: Arc<Mutex<HashMap<String, ChildSlot>>>,
}

impl PopupTransport {
    /// 未启用：dispatch/settle 全部 no-op（测试与非桌面环境默认）。
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn enabled() -> Self {
        Self {
            enabled: true,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 为一条发往 human 的消息拉起弹窗。
    /// surface 列表不含 popup、未启用或该消息已有弹窗在飞行中时跳过。
    pub fn dispatch(&self, storage: &Storage, msg: &Message, surfaces: &[String]) {
        let Some(slot) = self.claim(&msg.id, surfaces) else {
            return;
        };
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
            Ok(child) => {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                let in_flight = Arc::clone(&self.in_flight);
                let message_id = msg.id.clone();
                // ChildMonitor：try_wait 短轮询（不阻塞持锁），子进程退出后释放名额；
                // 无回复关闭即 dismissed，仅日志；settle 杀进程也在此回收。
                std::thread::spawn(move || loop {
                    let exited = {
                        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
                        match guard.as_mut() {
                            Some(child) => match child.try_wait() {
                                Ok(Some(status)) => {
                                    tracing::info!(
                                        message_id = %message_id,
                                        exit = %status,
                                        "popup window closed"
                                    );
                                    true
                                }
                                Ok(None) => false,
                                Err(e) => {
                                    tracing::warn!(
                                        message_id = %message_id,
                                        "popup monitor failed: {}", e
                                    );
                                    true
                                }
                            },
                            None => true,
                        }
                    };
                    if exited {
                        in_flight
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&message_id);
                        return;
                    }
                    std::thread::sleep(MONITOR_POLL);
                });
            }
            Err(e) => {
                self.release(&msg.id);
                tracing::warn!("popup spawn failed: {}", e);
                self.mark_failed(storage, msg, &e.to_string());
            }
        }
    }

    /// 抢答收尾：消息被他端（feishu/GUI/API）处理后关闭本端弹窗。
    /// 返回 true 表示确有在途弹窗并被关闭；弹窗进程的回收由 ChildMonitor 完成。
    pub fn settle(&self, message_id: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let slot = self
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(message_id)
            .cloned();
        let Some(slot) = slot else {
            return false;
        };
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(child) => {
                let _ = child.kill();
                tracing::info!(message_id = %message_id, "popup 已由其他端处理，关闭弹窗");
                true
            }
            None => false,
        }
    }

    /// 占名额：满足启用 + surface 含 popup + 该消息无在途弹窗时插入占位槽并返回。
    /// 占位槽在 spawn 成功后填入子进程句柄（供 settle 关闭）。
    fn claim(&self, message_id: &str, surfaces: &[String]) -> Option<ChildSlot> {
        if !self.enabled || !surfaces.iter().any(|s| s == POPUP_SURFACE) {
            return None;
        }
        let mut map = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(message_id) {
            return None;
        }
        let slot = Arc::new(Mutex::new(None));
        map.insert(message_id.to_string(), Arc::clone(&slot));
        Some(slot)
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
        assert!(t.claim("m1", &surfaces_with_popup()).is_none());
        assert!(!t.settle("m1"));
    }

    #[test]
    fn surface_without_popup_never_claims() {
        let t = PopupTransport::enabled();
        assert!(t.claim("m1", &["feishu".to_string()]).is_none());
    }

    #[test]
    fn claim_dedups_in_flight_message() {
        let t = PopupTransport::enabled();
        assert!(t.claim("m1", &surfaces_with_popup()).is_some());
        // 同一消息已有弹窗在途：不重复弹
        assert!(t.claim("m1", &surfaces_with_popup()).is_none());
        // 不同消息正常弹
        assert!(t.claim("m2", &surfaces_with_popup()).is_some());
        // 弹窗关闭（release）后同消息可再次弹
        t.release("m1");
        assert!(t.claim("m1", &surfaces_with_popup()).is_some());
    }

    #[test]
    fn settle_without_in_flight_is_noop() {
        let t = PopupTransport::enabled();
        assert!(!t.settle("no-such-msg"));
        // 占位槽尚未填入子进程（spawn 进行中）也不能 settle
        let _slot = t.claim("m1", &surfaces_with_popup()).unwrap();
        assert!(!t.settle("m1"));
    }

    #[cfg(unix)]
    #[test]
    fn settle_kills_in_flight_child() {
        let t = PopupTransport::enabled();
        let slot = t.claim("m1", &surfaces_with_popup()).unwrap();
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);

        assert!(t.settle("m1"));

        // 子进程被杀：wait 回收后状态非成功
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        let status = guard.as_mut().unwrap().wait().unwrap();
        assert!(!status.success());
        drop(guard);
        t.release("m1");
    }
}
