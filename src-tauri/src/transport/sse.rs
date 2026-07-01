//! SSE stream 生成：认证由调用方（server/http.rs）负责，本模块只做重放 + 实时推送。

use crate::routing::inbox;
use crate::routing::Message;
use crate::storage::Storage;
use crate::transport::wake::SseEvent;
use async_stream::stream;
use axum::response::sse::Event;
use std::convert::Infallible;
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt};

/// 面向 axum handler 的 SSE stream（已封装为 Event）。
pub fn events_stream(
    storage: Storage,
    address: String,
    last_event_id: Option<i64>,
    rx: broadcast::Receiver<SseEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    events_stream_raw(storage, address, last_event_id, rx)
        .map(|evt| Ok(event_from_message(&evt.message)))
}

/// 原始 stream：便于测试直接拿到 SseEvent。
/// `last_event_id` 为 `None` 表示客户端未带 Last-Event-ID，从当前最大 event_id 开始不重放历史；
/// `Some(id)` 用于断线续传，从 `id` 之后重放。
pub fn events_stream_raw(
    storage: Storage,
    address: String,
    last_event_id: Option<i64>,
    rx: broadcast::Receiver<SseEvent>,
) -> impl Stream<Item = SseEvent> {
    stream! {
        let mut rx = rx;
        let mut cursor = match last_event_id {
            Some(id) => id,
            None => inbox::max_event_id(&storage, &address).unwrap_or(0),
        };

        if let Ok(messages) = inbox::events_since(&storage, &address, cursor) {
            for msg in messages {
                cursor = msg.event_id;
                yield SseEvent { message: msg };
            }
        }

        while let Ok(evt) = rx.recv().await {
            // 去重：重放期间可能已拿到同一条消息。
            if evt.message.event_id > cursor {
                cursor = evt.message.event_id;
                yield evt;
            }
        }
    }
}

fn event_from_message(msg: &Message) -> Event {
    Event::default()
        .id(msg.event_id.to_string())
        .event("message")
        .data(serde_json::to_string(msg).unwrap_or_default())
}
