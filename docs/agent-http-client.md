# agtalk HTTP Agent 客户端接入指南

本文档面向希望通过 HTTP 长轮询接入 agtalk daemon 的外部 Agent / 脚本。

## 适用场景

- 外部 AI Agent（Codex / Claude / Kimi 等）需要异步接收 agtalk 消息
- 本地自动化脚本需要监听并处理任务、审批、指令
- 不想通过 Unix socket 或终端 transport，而是直接用 HTTP 通信

核心循环：

```text
poll → receive → read → process → reply/done → poll
```

## 重要协议约定

agtalk 的 HTTP 接口**不是 REST API**，所有语义都通过 `POST /api` 发送 `ClientMsg` JSON 实现，与 Unix socket 协议完全一致。

```text
POST /api
Content-Type: application/json
X-Agtalk-Session-Id: <session_id>
X-Agtalk-Token: <token>
```

## 前置条件

1. daemon 已启动：

   ```bash
   agtalk daemon start
   ```

2. Agent 已加入 workspace：

   ```bash
   agtalk join <agent-name> --role agent
   ```

3. 本地 session 文件已生成：

   ```text
   .agtalk/sessions/<agent-name>.json
   ```

   其中包含：

   - `session_id`
   - `token`
   - `name`
   - `workspace_id`

4. HTTP 地址（默认）：

   ```text
   http://127.0.0.1:19527/api
   ```

   端口由 `daemon.http_port` 配置决定，可通过 `agtalk config get daemon.http_port` 查看。

## 鉴权

`PollInbox` 属于读消息正文的敏感接口，**必须**提供：

- `X-Agtalk-Session-Id`
- `X-Agtalk-Token`

**不允许**使用 `X-Agtalk-Name` fallback。

缺少 token 时返回：

```json
{
  "type": "error",
  "code": "poll_inbox_requires_session_token",
  "message": "PollInbox 必须提供 X-Agtalk-Session-Id 和 X-Agtalk-Token"
}
```

## PollInbox 请求

### 请求体

```json
{
  "type": "poll_inbox",
  "filter": "unread",
  "timeout_ms": 30000,
  "limit": 10
}
```

### 字段说明

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `type` | string | 是 | — | 固定为 `poll_inbox` |
| `filter` | string | 否 | `unread` | `unread` / `pending` / `action_required` / `all` |
| `timeout_ms` | number | 否 | `30000` | 最长等待毫秒数，默认 30 秒，最大可设为 10 分钟（`600000`） |
| `limit` | number | 否 | `10` | 返回条数上限，最大 `50` |

`timeout_ms` 建议根据 Agent 调度策略设置：高频短轮询用默认 30 秒；低频次后台 Agent 可设 1～10 分钟，减少空轮询开销。

### curl 快速验证

```bash
SESSION_ID=$(jq -r .session_id .agtalk/sessions/<agent-name>.json)
TOKEN=$(jq -r .token .agtalk/sessions/<agent-name>.json)

curl -s -X POST http://127.0.0.1:19527/api \
  -H "X-Agtalk-Session-Id: $SESSION_ID" \
  -H "X-Agtalk-Token: $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "type": "poll_inbox",
    "filter": "unread",
    "timeout_ms": 5000,
    "limit": 10
  }'
```

## PollInbox 返回

### 成功返回

```json
{
  "type": "ok",
  "data": {
    "messages": [...],
    "timed_out": false,
    "empty": false,
    "limit": 10,
    "timeout_ms": 30000
  }
}
```

### 字段说明

| 字段 | 说明 |
|------|------|
| `messages` | 本次查询到的 inbox 消息列表 |
| `timed_out` | `true` 表示本轮等到超时仍无消息 |
| `empty` | `true` 表示 `messages` 为空 |
| `limit` / `timeout_ms` | 实际生效值 |

### 超时无消息返回

```json
{
  "type": "ok",
  "data": {
    "messages": [],
    "timed_out": true,
    "empty": true,
    "limit": 10,
    "timeout_ms": 30000
  }
}
```

## 消息处理流程

标准循环：

```python
while True:
    result = poll_inbox(session_id, token, timeout_ms=30000, limit=10)

    if result["empty"]:
        continue

    for msg in result["messages"]:
        # 1. 标记已读（可选但建议）
        read_message(session_id, token, msg["id"])

        # 2. 处理消息
        response = handle_message(msg)

        # 3. 回复或标记完成
        if response:
            send_reply(session_id, token, msg["id"], response)
        else:
            mark_done(session_id, token, msg["id"])
```

### 关键语义

- `PollInbox` 返回前会把 `pending` 消息标记为 `delivered`
- `PollInbox` **不会**自动 `read` / `done`
- `unread` filter 包含 `pending` 和 `delivered`，保证 at-least-once delivery
- Agent 崩溃后重启，未 `read/done` 的消息仍可被后续 poll 取到

## 常用后续操作

### 标记已读

```json
{
  "type": "read",
  "msg_id": "<msg-id>",
  "participant": "<agent-name>"
}
```

### 回复消息

```json
{
  "type": "reply",
  "msg_id": "<msg-id>",
  "choice": "ok",
  "reason": "已完成"
}
```

### 标记完成

```json
{
  "type": "done",
  "msg_id": "<msg-id>",
  "participant": "<agent-name>"
}
```

## 错误处理

| 错误码 | 含义 | 建议处理 |
|--------|------|----------|
| `poll_already_active` | 同一 participant 已有挂起 poll | 检查是否有其他进程在 poll，或等待当前 poll 结束 |
| `poll_inbox_requires_session_token` | 缺少 session_id/token | 检查 headers 是否正确 |
| `auth_failed` / `auth_required` | 鉴权失败 | 检查 session 是否有效、是否被接管 |
| `timed_out=true` | 本轮无消息 | 正常情况，继续下一轮 poll |

## Python 最小示例

完整可运行脚本：[examples/http_agent_poll.py](../examples/http_agent_poll.py)

同时已作为 memory 沉淀在 `http-agent` topic 下：

```bash
agtalk mem show 64d85564
```

核心片段：

```python
import json
import requests

SESSION_FILE = ".agtalk/sessions/my-agent.json"
BASE_URL = "http://127.0.0.1:19527/api"

with open(SESSION_FILE) as f:
    sess = json.load(f)

headers = {
    "X-Agtalk-Session-Id": sess["session_id"],
    "X-Agtalk-Token": sess["token"],
    "Content-Type": "application/json",
}

def call(msg):
    r = requests.post(BASE_URL, headers=headers, json=msg)
    r.raise_for_status()
    return r.json()

while True:
    resp = call({
        "type": "poll_inbox",
        "filter": "unread",
        "timeout_ms": 30000,
        "limit": 10,
    })

    data = resp.get("data", {})
    for msg in data.get("messages", []):
        print(f"收到消息: {msg['id']} - {msg.get('body', '')[:100]}")
        call({"type": "read", "msg_id": msg["id"], "participant": sess["name"]})
        call({"type": "done", "msg_id": msg["id"], "participant": sess["name"]})
```

## 注意事项

- 同一 participant 同时只能有一个挂起的 `PollInbox`，否则返回 `poll_already_active`
- 建议每个 Agent 使用独立进程循环 poll，不要多进程共用一个 session
- 如果 Agent 需要同时处理多个 workspace，应分别 join 并管理多个 session
- 当前版本只支持本地 HTTP（`127.0.0.1`），不开放公网访问

## 后续规划

当前 `PollInbox` 第一版已足够支撑外部 Agent 异步接入。后续如果长轮询在真实接入中出现性能或体验问题，再评估是否需要 WebSocket / SSE。
