// SSE demo：daemon 有新内容就主动推，客户端零轮询。
// 启动后：
//   终端 A：cargo run                 (daemon)
//   终端 B：curl -N localhost:3000/events          (订阅 SSE，阻塞等待)
//   终端 C：curl -X POST localhost:3000/publish -d 'hello'   (产生新内容)
// 终端 B 会立刻收到 "hello"，证明是事件驱动推送，不是轮询。

use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    routing::{get, post},
    Router,
};
use std::convert::Infallible;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

// 全局共享：一个广播 channel，任何 publish 都会瞬间唤醒所有订阅者。
#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() {
    // capacity=64 只是缓冲；订阅慢了就丢旧的——真实场景用 cursor + DB 重放来保证不丢。
    let (tx, _rx) = broadcast::channel::<String>(64);
    let state = AppState { tx };

    let app = Router::new()
        .route("/events", get(sse_events))   // 订阅：阻塞，有内容才推
        .route("/publish", post(publish))    // 产生新内容：瞬间唤醒所有订阅者
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("daemon: http://127.0.0.1:3000");
    println!("  订阅:  curl -N http://127.0.0.1:3000/events");
    println!("  推送:  curl -X POST http://127.0.0.1:3000/publish -d '你好'");
    axum::serve(listener, app).await.unwrap();
}

/// SSE 订阅端点：连接后阻塞，只有 tx.send() 才会唤醒它推一条出去。
async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tx.subscribe();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let stream = BroadcastStream::new(rx).filter_map(move |res| {
        let msg = res.ok()?;
        // 给每条事件一个单调 id——客户端断线重连时用 Last-Event-ID 续传。
        let id = now; // demo 用时间戳简化；真实场景用 DB 的自增 message_recipients id
        Some(Ok(Event::default().id(id.to_string()).event("message").data(msg)))
    });

    // KeepAlive: 长时间没消息时发心跳，防止中间代理掐断连接。
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 产生新内容：写一行日志 + 广播。所有订阅的 SSE 连接会在毫秒内收到。
async fn publish(
    State(state): State<AppState>,
    body: String,
) -> impl IntoResponse {
    let body = body.trim().to_string();
    println!("[daemon] 收到新内容，主动推送给所有订阅者: {:?}", body);
    // 关键：这里就是"有内容后再执行"——send 会唤醒所有阻塞在 subscribe() 上的 SSE 流。
    let _ = state.tx.send(body.clone());
    (StatusCode::OK, format!("published: {}\n", body))
}
