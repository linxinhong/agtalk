//! 图事件推送中枢（M3，docs/design_graph.md §7）。
//!
//! 按 graph_run_id 维护 broadcast channel：图状态变化（落库后）由 handler 层
//! flush 推送（design_graph.md §11 红线 6：先落库再推 SSE）；订阅端支持
//! Last-Event-ID 断线重放。

use crate::graph::dto::GraphEventDto;
use crate::graph::events;
use crate::storage::Storage;
use async_stream::stream;
use axum::response::sse::Event;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio_stream::Stream;

#[derive(Clone)]
pub struct GraphEventHub {
    senders: Arc<Mutex<HashMap<String, broadcast::Sender<GraphEventDto>>>>,
}

impl Default for GraphEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphEventHub {
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self, run_id: &str) -> broadcast::Receiver<GraphEventDto> {
        let mut senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        let sender = senders
            .entry(run_id.to_string())
            .or_insert_with(|| broadcast::channel(128).0);
        sender.subscribe()
    }

    pub fn push(&self, run_id: &str, event: GraphEventDto) {
        let senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = senders.get(run_id) {
            let _ = sender.send(event);
        }
    }

    pub fn subscriber_count(&self) -> usize {
        let senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        senders.values().map(|s| s.receiver_count()).sum()
    }
}

/// 面向 axum handler 的 GraphEvent SSE stream。
/// `last_event_id` 为 None 时不重放历史（从当前最大 id 起）；Some(id) 断线续传。
pub fn graph_events_stream(
    storage: Storage,
    run_id: String,
    last_event_id: Option<i64>,
    rx: broadcast::Receiver<GraphEventDto>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream! {
        let mut rx = rx;
        let mut cursor = match last_event_id {
            Some(id) => id,
            None => max_graph_event_id(&storage, &run_id),
        };
        // 重放：先落库后推送，断线不丢（design_graph.md §11 红线 6）
        let replay = replay_after(&storage, &run_id, cursor);
        for evt in replay {
            cursor = evt.id;
            yield Ok(event_from_dto(&evt));
        }
        while let Ok(evt) = rx.recv().await {
            if evt.id > cursor {
                cursor = evt.id;
                yield Ok(event_from_dto(&evt));
            }
        }
    }
}

fn max_graph_event_id(storage: &Storage, run_id: &str) -> i64 {
    let conn = storage.conn();
    events::max_event_id(&conn, run_id).unwrap_or(0)
}

fn replay_after(storage: &Storage, run_id: &str, cursor: i64) -> Vec<GraphEventDto> {
    let conn = storage.conn();
    events::list_after(&conn, run_id, Some(cursor), 500)
        .unwrap_or_default()
        .into_iter()
        .map(|e| GraphEventDto {
            id: e.id,
            event_type: e.event_type,
            node_key: e.node_key,
            payload: e.payload,
            created_at: e.created_at,
        })
        .collect()
}

fn event_from_dto(e: &GraphEventDto) -> Event {
    Event::default()
        .id(e.id.to_string())
        .event("graph")
        .data(serde_json::to_string(e).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_pushes_to_subscriber() {
        let hub = GraphEventHub::new();
        let mut rx = hub.subscribe("g1");
        let evt = GraphEventDto {
            id: 1,
            event_type: "graph_created".into(),
            node_key: None,
            payload: serde_json::json!({}),
            created_at: 0.0,
        };
        hub.push("g1", evt.clone());
        let got = rx.try_recv().unwrap();
        assert_eq!(got.id, 1);
        assert_eq!(got.event_type, "graph_created");
        // 其他 run 不串扰
        let mut rx2 = hub.subscribe("g2");
        assert!(rx2.try_recv().is_err());
    }
}
