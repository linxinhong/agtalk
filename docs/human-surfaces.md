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
POST /api/v1/human/delivery/ack             # {message_id, surface} delivery 回执（surface 确认已展示）
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
- 桌面 popup（已实现）：daemon 的 PopupTransport 在 fanout 成功后拉起 `agtalk __popup <message-id>`
  （in-flight 去重，spawn 失败标 failed）；弹窗展示成功后经 `POST /api/v1/human/delivery/ack`
  回执 delivered。v1 只在 fanout 时拉起，daemon 启动/backlog 不补弹。
- 后续外部通道（飞书等）经 `deliver_via` 投递：成功写 external_ref 标 delivered；失败记
  error 并 attempts+1，failed 行可被重试捞取。
- delivery 失败不影响消息投递：消息始终在 DB，human inbox/SSE 可读（at-least-once）。

## 6. 跨端事件幂等

外部 surface（飞书消息回执、Android command）回写动作时带
`{surface, external_event_id}`。reply / done / send 三个动作统一幂等：

- 同一 `(surface, external_event_id)` 只执行一次；重复事件**回放首次的成功结果**
  （`Ok{id}`，send 返回原 message id），不重复创建消息、不重复触发 SSE/notify。
- **同事务原子**：动作预生成结果消息 id，receipt 携带该 id 与业务写入在同一事务提交——
  receipt 存在 ⟺ 结果消息存在，不存在"占位 receipt"崩溃窗口；事务内任何失败整体回滚，
  receipt 无残留，外部可修正后重试同一事件。
- 历史占位数据（receipt 无结果 id，无法确定原动作是否已落库）返回
  `receipt_inconclusive`，不盲目重放/重试。

## 7. 飞书 surface（内置 transport）

飞书是 daemon 内置的 human transport（不是 notify plugin），启用方式：

```bash
agtalk config set feishu.enabled true
agtalk config set feishu.app_id <app_id>
agtalk config set feishu.app_secret <app_secret>
agtalk config set feishu.open_id <绑定用户的 open_id>
agtalk config set human.surfaces '["popup","feishu"]'
```

也可在 `agtalk config gui` 的「飞书」卡片中编辑（secret 字段密码框，surfaces 按 JSON 数组编辑）。
配置存 `<config_dir>/config.json`（0600），不进 sqlite；sqlite 只存消息/身份/delivery/receipt 等运行时数据。

行为：

- **出站**：fanout 含 feishu surface 时，FeishuDispatcher 经长连接机器人发**交互卡片**
  （approval_request 渲染 choices 按钮，普通消息为文本卡片）；卡片发送失败回退纯文本
  `[agtalk] {from}: {body}`；再失败经 `deliver_via` 标 failed（attempts+1，可重试）。
  成功标 delivered，external_ref = open_message_id。
- **入站**：FeishuRouter 经飞书长连接（websocket）接收卡片回调与消息事件；
  只有 `feishu.open_id` 绑定用户的点击/消息生效，其他人操作直接忽略（v1 单用户）。
  卡片回调按动作分派：
  - `approval`：按钮 value 携带 `{agtalk_msg, choice_index}` 精确路由审批回复（仲裁语义不变）。
  - `compose_submit`：从 `action.form_value` 读取正文与目标 agent；**服务端必须再次验证**
    目标 UUID 是活跃 agent（不信任卡片 payload），然后复用 human→agent 发信与 receipt 幂等路径。
  - `reply_open` / `reply_submit`：普通文本卡片的「回复」入口——卡片原地切换为回复表单
    （目标锁定原发送 agent），提交后复用 human reply 路径，保留 reply_to_id、SSE、notify、history。
  - 绑定用户的 **p2p 文本消息**回复「选择 Agent 并发送」草稿卡：正文预填为该文本，
    agent 下拉展示 name + intro（option value 只放 address UUID，完整 UUID 不进可见文案）；
    无可投递 agent 时回说明文本，不生成空选择卡片。
  - 群聊、非文本、空文本安全忽略；不做群聊路由，不做自由文字归属猜测。
- **幂等**：飞书 event_id 作为 external_event_id 走第 6 节同事务幂等；
  重复事件回放终态卡片（视觉收敛），不重复创建消息、不重复触发 SSE/notify。
- **仲裁收尾**：审批被其他 surface（popup/GUI）处理时，daemon 回写飞书卡片为终态
  「已由 {surface} 处理」；飞书自己胜出时由 Router 直接回终态。
- **可观测**：`agtalk tool doctor` 输出 feishu 段（enabled/凭据存在性脱敏/open_id 绑定/
  surfaces 包含 feishu）；长连接状态见 daemon 日志。

detect 向导已实现为 GUI 一键创建：`agtalk config gui` →「飞书」卡片 →「一键创建应用」，走飞书开放平台 OAuth 设备授权流（RFC 8628）：打开授权链接 → 飞书中确认（应用按最小权限创建：`im:message:send_as_bot` / `im:message:update` / `im:message.p2p_msg:readonly` + 事件 `im.message.receive_v1` + 回调 `card.action.trigger`）→ 自动写入 `feishu.app_id` / `feishu.app_secret` / `feishu.open_id`（扫码用户的 open_id，无需手工发现）/ `feishu.enabled=true`，并把 `feishu` 并入 `human.surfaces`。重启 daemon 后生效。缺权限时在开发者后台手工补。

## 8. 错误码

| code | 含义 |
|---|---|
| `auth_failed` | 缺/错 human token，或 session 与 mailbox 不一致 |
| `inbox_empty` | 无未读消息（非 0 退出，不是失败） |
| `message_not_found` | 消息不存在或不属于 human |
| `message_id_too_short` / `message_id_ambiguous` | 短 ID 非法/歧义 |
| `already_resolved` | 审批已被其他 surface 处理 |
| `select_only_requires_choice` | select_only 审批不允许自由文本 |
| `invalid_choice` | choice 不在审批选项中 |
| `agent_not_found` | send 目标不存在、已离开或为 human 自身 |
| `receipt_inconclusive` | 历史占位 receipt 无结果 id，无法确定原动作是否已落库（需人工清理该 receipt 后重试） |
| `delivery_not_found` | delivery ack 的 message_id+surface 无对应 delivery 行 |
| `invalid_surface` | delivery ack 的 surface 为空 |

## 9. surface 实现 checklist

1. 读 `<config_dir>/human/session.json` 取 address + token（0600，失败提示重启 daemon）。
2. `GET /api/v1/events` 带 token 订阅 SSE，断线用 Last-Event-ID 重放。
3. 收到 approval_request 渲染 choices；select_only 只允许选项点击。
4. 回复走 `POST /api/v1/human/reply`，处理 `already_resolved`（禁用该消息的操作按钮）。
5. 主动发信前先 `GET /api/v1/human/agents` 刷新列表，只发列表内 UUID。
6. 任何 API 401：重新读 session.json；仍失败提示用户重启 daemon。
