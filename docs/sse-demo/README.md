# SSE Demo —— agtalk 推送机制的参考实现

> 这不是产品代码，是验证「daemon 有内容主动通知 agent、零轮询」的最小可运行 demo。
> 对应设计文档：`docs/design.md` 第 4 节（推送机制 SSE）。
> agtalk 主项目开发时可参考 `~/projects/agtalk-office` 中 PollInbox 的唤醒思想（poll_waiters）。

## 它验证了什么

1. **事件驱动，零轮询**：SSE 连接平时阻塞 `await`，只有 `publish` 主动 `tx.send()` 才推送。订阅后静默期间客户端收到 NOTHING（不是定时返回）。
2. **毫秒级延迟**：从 `tx.send` 到客户端收到，是单次 channel send + 单次 TCP write。
3. **唤醒机制可复用**：这正是 agtalk 里 `handle_send` 写完消息后「唤醒订阅了该 UUID 的 SSE 连接」的核心模式。agtalk-office 早期用 PollInbox 的 `poll_waiters` 实现了同样的唤醒思想，本 demo 是其「不断开连接」版本。

## 这个 demo 没覆盖、agtalk 必须补的

demo 用 `broadcast::channel(64)`，订阅慢了/断线会丢消息。agtalk 生产实现必须补：
- 消息推送前**先持久化**到 mailbox 收件箱（at-least-once）
- 单调递增 `event_id`（用 DB 行）
- 客户端断线重连带 `Last-Event-ID` → daemon 从 DB 重放 `event_id > last` 的消息
- 按 `address(UUID)` 订阅过滤（demo 是广播给所有订阅者，agtalk 要按 UUID 路由）
- 认证（demo 无鉴权，agtalk 要校验 PID + session.json，见 design.md 2.3）

## 怎么跑

```bash
cd docs/sse-demo
cargo run --release                    # 启动 daemon，监听 127.0.0.1:3000

# 终端 B：订阅 SSE（阻塞等待，-N 关闭 curl 缓冲）
curl -N http://127.0.0.1:3000/events

# 终端 C：产生新内容 —— B 端会瞬间收到
curl -X POST http://127.0.0.1:3000/publish -d '你好'
curl -X POST http://127.0.0.1:3000/publish -d '再来一条'
```

## 文件

- `src/main.rs` —— axum + tokio broadcast，< 90 行。`/events` 订阅、`/publish` 产生内容并唤醒。
- `Cargo.toml` —— 依赖：axum 0.7、tokio (full)、tokio-stream (sync feature)、serde_json。
