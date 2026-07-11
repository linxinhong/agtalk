# human-surfaces.md — human surface 接入协议

本文档面向 human surface（popup / GUI / 浏览器扩展内的人类面板 / 飞书 / Android）的实现者。
架构设计见 `docs/design.md` §3.6，命令面见 `docs/commands.md` §11.5。

## 1. 定位与红线

- **daemon 是唯一真相来源**。surface 是薄客户端：所有读写经 `/api/v1/human/*` 与
  `GET /api/v1/events`，**禁止直写 SQLite**。
- human mailbox 是普通 mailbox（`system_mailboxes` role='human'），路由仍只认 UUID。
- human token 是 design §2.3 的第三种受限认证例外，**仅限本机 human 客户端**，
  绝不暴露给 agent：agent 没有任何命令可读取它。

## 2. 认证

daemon 启动时在 `<config_dir>/human/session.json`（目录 0700、文件 0600）颁发
system-human session：

```json
{ "version": 1, "address": "<human-uuid>", "name": "human", "token": "<uuid>", "created_at": "..." }
```

- macOS/Linux config_dir：`~/.config/agtalk2`。
- 本机 surface 直接读该文件取 token；token 失效（address 变化重发）时重启 daemon 或重新读取。
- 所有 human API 与 SSE 订阅带 header：

```text
X-AgTalk-Human-Token: <token>
```

- 缺 token / 错误 token / token 与 DB human 地址不一致：`401` + `auth_failed`。

## 3. API 一览

```text
GET  /api/v1/human/inbox?all=true|false     # human 收件箱
POST /api/v1/human/read                     # {message_id?} 读未读/指定消息并标 read
POST /api/v1/human/reply                    # {message_id, body, choice?, surface?, external_event_id?}
POST /api/v1/human/done                     # {message_id} 标记完成
GET  /api/v1/human/agents                   # 在线 agent 列表（活跃 mailbox，排除 human 自身）
POST /api/v1/human/send                     # {to, body, subject?} 主动发信（to 必须是 agents 列表中的 UUID）
GET  /api/v1/events                         # SSE 订阅 human mailbox（带 human token + 可选 Last-Event-ID）
```

- 消息 id 支持短前缀（≥ 8 位），歧义返回 `message_id_ambiguous`。
- 所有响应是 `ServerMsg` JSON envelope（`{"type": ...}`），错误为
  `{"type": "error", "code": "...", "message": "..."}`。

## 4. 消息类型与审批仲裁

发给 human 的消息有两种 content_type：

- `text`：普通询问/通知。**允许多次 reply**；首次 reply 把原消息 pending → read。
- `approval_request`：审批请求（`msg ask --option ...` 产生），metadata 形如
  `{"choices": [...], "recommended": ..., "single": bool, "select_only": bool}`。

回复语义（`POST /api/v1/human/reply`）：

- approval_request **首个有效回复原子胜出**：单事务内写回复消息 + approval_resolutions +
  原消息置 done，并发多 surface 只有一个成功。
- 后续回复返回 `already_resolved`（message 中含胜出 surface 与回复消息 id）。
- `select_only=true` 时必须带 `choice`（且属于 metadata.choices），否则
  `select_only_requires_choice`；choice 不在选项中返回 `invalid_choice`。
- `select_only=false` 允许纯文本回复（不带 choice）。
- 回复消息 content_type 为 `approval_response`（审批）或 `text`，经 SSE/notify 送达原发送方。

## 5. delivery 状态机（surface 可观测性）

每条发给 human 的消息按 `config.human.surfaces`（默认 `["popup"]`）fanout，为每个 surface
写一条 `human_deliveries` 记录：

```text
pending → delivered（surface 确认收到）/ failed（可重试）
```

- phase-1：daemon 只做**记账**（消息落库 + delivery 行 + SSE 推送），surface 经 SSE 拉取消息。
- 后续外部通道（飞书等）经 `deliver_via` 投递：成功写 external_ref 标 delivered；失败记
  error 并 attempts+1，failed 行可被重试捞取。
- delivery 失败不影响消息投递：消息始终在 DB，human inbox/SSE 可读（at-least-once）。

## 6. 跨端事件去重

外部 surface（飞书消息回执、Android command）回写动作时带
`{surface, external_event_id}`：

- 同一 `(surface, external_event_id)` 只处理一次，重复返回 `duplicate_event`。
- 去重与业务写入在同一事务内，保证原子。

## 7. 错误码

| code | 含义 |
|---|---|
| `auth_failed` | 缺/错 human token，或 session 与 mailbox 不一致 |
| `inbox_empty` | 无未读消息（非 0 退出，不是失败） |
| `message_not_found` | 消息不存在或不属于 human |
| `message_id_too_short` / `message_id_ambiguous` | 短 ID 非法/歧义 |
| `already_resolved` | 审批已被其他 surface 处理 |
| `select_only_requires_choice` | select_only 审批不允许自由文本 |
| `invalid_choice` | choice 不在审批选项中 |
| `duplicate_event` | 重复的跨端事件 |
| `agent_not_found` | send 目标不存在或已离开 |

## 8. surface 实现 checklist

1. 读 `<config_dir>/human/session.json` 取 address + token（0600，失败提示重启 daemon）。
2. `GET /api/v1/events` 带 token 订阅 SSE，断线用 Last-Event-ID 重放。
3. 收到 approval_request 渲染 choices；select_only 只允许选项点击。
4. 回复走 `POST /api/v1/human/reply`，处理 `already_resolved`（禁用该消息的操作按钮）。
5. 主动发信前先 `GET /api/v1/human/agents` 刷新列表，只发列表内 UUID。
6. 任何 API 401：重新读 session.json；仍失败提示用户重启 daemon。
