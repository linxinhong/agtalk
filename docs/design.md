# agtalk 设计文档

> 版本：v1（初版设计）｜ 日期：2026-07-01
> 状态：设计已定，待实现计划

---

## 1. 项目背景与定位

**agtalk 是一个本地 Agent 对话总线**——让 CLI agent、网页 AI、GUI、人类之间可靠传话、追踪投递状态、保存上下文的本地通信中枢。daemon 是所有状态的唯一真相来源，CLI / GUI / 浏览器扩展都是薄客户端。

**核心场景**：
- agent ↔ agent：多个本地 agent 交换结构化任务消息
- agent ↔ human：agent 执行风险操作前向人类请求确认（Human-in-the-loop）
- agent ↔ browser：浏览器扩展把网页 AI（ChatGPT/Claude 等）桥接到 agtalk 总线
- agent ↔ mobile：Android Deck 通过 BLE 作为移动控制台，处理 Deck / Notify / Inbox / Reply / Done

**本项目参考自 `~/projects/agtalk-office`**。agtalk-office 是同一作者的早期实现，验证了"daemon + IPC + SQLite + Tauri GUI + 浏览器扩展"的可行性，也沉淀了若干设计教训（身份爆炸、令牌在 agent compact 时丢失、轮询机制冗余等）。**agtalk 是全新项目**，不复用 agtalk-office 的代码，但借鉴其设计思路（intro/workspace 字段、notify 插件、YAML Runner、扩展前端架构）。读者可在 agtalk-office 中看到这些思路的原始形态。

---

## 2. 核心设计决策

### 2.1 身份/路由分离

```
address : UUID      ← 路由唯一锚（机器用，进所有路由查询，唯一）
name    : 字符串    ← 可读展示名（人看，不唯一，不进路由查询）
```

- **路由只认 UUID**。没有按 name 的路由匹配、没有模糊查找、没有消歧分支。
- **name 可以重复**。多个 agent 可以都叫 `nora`，互不冲突。
- 查询接口返回带元数据的候选列表，消歧发生在**调用方**（agent 用 LLM 推理选 UUID）。

### 2.2 身份载体 = 工作目录的文件系统

```
<工作目录>/.agtalk/
  ├─ agents.json                 ← pid → name 映射（进程查"我是谁"）
  │     { "12345": {"name":"nora","start_time":1782...}, ... }
  ├─ nora/
  │    └─ session.json           ← nora 的身份与认证材料
  └─ quinn/
       └─ session.json           ← quinn 的身份与认证材料
```

- 同一工作目录可并存多个 agent，每个一个 `<name>/session.json` 子目录。
- `agents.json` 提供 `pid → name` 映射，让进程能自查身份（多 agent 并存时知道读哪个 session 文件）。
- session.json 权限 0600。

**session.json 内容**：
```json
{
  "address": "550e8400-e29b-41d4-a716-446655440000",
  "name": "nora",
  "workspace": "projA",
  "intro": "前端 review",
  "created_at": "2026-07-01T..."
}
```

**session.json 不含高熵 token。** 认证不依赖"agent 出示一个秘密字符串"，而依赖**文件系统本身就是信任根**——能读到这个文件的进程（同 uid、同工作目录）即被信任持有该身份。daemon 侧再叠加 PID + start_time 校验防 PID 复用。

### 2.3 抗 compact 的认证链

```
agent 进程发请求时：
  ① getpid()                              ← 进程属性，compact 压不到
  ② 查 agents.json[pid] → name           ← 文件系统，压不到
  ③ 读 .agtalk/<name>/session.json        ← 文件系统，压不到
  ④ 取出 address(UUID)，以该身份发请求     ← 请求带 address + PID
  ⑤ daemon 校验：PID 的 start_time 与 agents.json 一致 + 文件确实存在
```

**全链基于 PID + 文件系统，agent 大脑里不需要存任何高熵字符串。** compact 之后重新执行 ①→④ 即恢复身份。身份载体放在 agent 丢不掉的地方（进程属性 + 磁盘文件），而不是 agent 最容易丢的地方（对话噪声里的高熵 token）。

> **浏览器扩展例外**：浏览器扩展无法读取本地 `.agtalk/` 文件系统，因此采用 daemon 颁发的 token 认证（见 §2.6）。该例外仅限浏览器域。
>
> **Android BLE 例外**：Android APK 同样无法读取本地 `.agtalk/` 文件系统，因此配对后采用 daemon 颁发的 mobile device token 认证（见 §2.7）。该例外仅限 Android BLE transport。
>
> 除浏览器扩展和 Android BLE 外，其它 CLI/GUI/agent-agent 域仍必须走 PID + 文件系统。

### 2.4 PID 复用防护

`agents.json` 存 `{pid, name, start_time}`。daemon 校验时比 `pid + 进程启动时间` 双因子。PID 被 OS 复用时 start_time 不一致，识别得出。

### 2.5 HTTP 接口

NG canonical REST API 按领域组织，版本前缀 `/api/v1`。

身份（id）：

```
POST /api/v1/id/join
Headers: X-AgTalk-Address, X-AgTalk-Pid, X-AgTalk-Start-Time
Body: {name?, intro?, workspace?, notify?}
→ {type: "identity", address: UUID, name: string, ...}

POST /api/v1/id/leave
Headers: X-AgTalk-Address, X-AgTalk-Pid, X-AgTalk-Start-Time
→ {type: "pong"}

GET /api/v1/id/me
Headers: X-AgTalk-Address, X-AgTalk-Pid, X-AgTalk-Start-Time
→ {type: "identity", address: UUID, name: string, ...}

GET /api/v1/id/lookup?name=nora  （或无参列全部）
→ {type: "lookup_result", mailboxes: [{address: UUID, name: "nora", intro: "前端 review", workspace: "projA"}, ...]}
```

消息（msg）：

```
POST /api/v1/msg/send
Headers: X-AgTalk-Address, X-AgTalk-Pid, X-AgTalk-Start-Time
Body: {to: UUID, body: string, content_type?, reply_to_id?, metadata?, more_coming?}
→ {type: "ok", id: msg_id}

POST /api/v1/msg/reply
Body: {message_id: string, body: string, ...}
→ {type: "ok", id: msg_id}

POST /api/v1/msg/done
Body: {message_id: string, body?: string, ...}
→ {type: "ok"}

POST /api/v1/msg/ask
Body: {body: string, questions?: [...], options?: [...], ...}
→ {type: "ok", id: msg_id}

GET /api/v1/msg/inbox?all=true|false
→ {type: "inbox", messages: [...]}

POST /api/v1/msg/read
Body: {message_id?: string}
→ 无 message_id 时返回未读列表；有 message_id 时返回单条详情

POST /api/v1/msg/wait
Body: {message_id?: string, timeout: number, since?: number}
→ 命中或超时返回

GET /api/v1/msg/attachment/:id
→ 附件二进制或元数据
```

记忆（mem）：

```
GET   /api/v1/mem/plan?address=...&name=...
PATCH /api/v1/mem/plan
Body: {plan?: string, context?: string, status?: string, summary?: string}
GET   /api/v1/mem/plan/status?address=...&name=...
```

推送：

```
GET /api/v1/events
Headers: X-AgTalk-Address, X-AgTalk-Pid, X-AgTalk-Start-Time, Last-Event-ID
→ SSE stream
```

浏览器扩展专用：

```
POST /api/v1/browser/join
Body: {name?, intro?, workspace?}
→ {type: "browser_join_result", address: UUID, name: string, token: string}
```

旧路径 `/api/join`、`/api/leave`、`/api/lookup`、`/api/send`、`/events` 及统一入口 `POST /api` 保留为内部兼容/调试别名，canonical API 使用上述 `/api/v1/*` 路径。

### 2.6 浏览器扩展认证

浏览器扩展无法访问 `.agtalk/` 文件系统，因此由 daemon 在首次连接时颁发 token：

```
扩展 → POST /api/v1/browser/join
       ← {address, name, token}

后续请求：
  Headers: X-AgTalk-Address: <address>
           X-AgTalk-Browser-Token: <token>
```

- token 为高熵 UUID，存储在 `chrome.storage.local`。
- daemon 在 `browser_sessions` 表中校验 token，并确认对应 mailbox 未 leave。
- token 与 mailbox 生命周期绑定：`POST /api/v1/id/leave` 删除 token 并 mark_left mailbox。
- 常驻 SSE 订阅走 `GET /api/v1/events`，携带 `X-AgTalk-Address` 与 `X-AgTalk-Browser-Token`。
- 该认证方式**仅限浏览器域**，不得用于 CLI/GUI/agent-agent 域。

### 2.7 Android BLE 移动端认证与 transport

Android APK 无法访问本地 `.agtalk/` 文件系统，因此 Android BLE transport 使用独立 mobile device token 认证。该 token 是浏览器扩展之外的第二个明确例外，范围只限 Android BLE transport，不得扩展到 CLI/GUI/agent-agent 域。

完整设计见 [`docs/android-ble-deck.md`](android-ble-deck.md)。

核心规则：

- Android APK = BLE Peripheral / GATT Server，广播 `agtalk-deck`。
- agtalk daemon = BLE Central / GATT Client，扫描并连接 Android。
- 连接配置、扫描、配对、设备管理、Deck 调试统一通过 `agtalk config gui`，不新增 `agtalk mobile ...` 或 `agtalk deck ...` 顶层命令。
- pairing 必须由本机用户在 GUI 中显式开启；未 trusted 设备只能 pairing，不能触发 action。
- 配对后 daemon 颁发 mobile device token，并在 SQLite 中保存 token hash 与 trusted device 记录。
- Android 命令必须携带 `device_id + token + command_id`，daemon 做 token 校验、幂等去重和审计。
- BLE 不新增业务消息模型，只把 `deck.trigger` / `message.reply` / `message.done` 转换成现有 agtalk msg/routing 操作。
- Deck 模板在 agtalk 本地展开，Android 只发送 `deck_id + button_id`。
- Android BLE v0.1 禁止 shell、git push、配置修改、删除文件和高风险 approval。

### 2.8 mem：计划、上下文与长期记忆

`mem` 分三层，边界清晰：

#### 2.8.1 内置 guide（二进制嵌入）

- 源文件：`docs/agent-usage.md`。
- 编译时通过 `include_str!` 嵌入二进制。
- 通过 `agtalk --agent-guide` 读取。
- 不写入任何 agent 本地 memory，不进入 `<config_dir>/memory/`，不进入 `agtalk.db`。

#### 2.8.2 agent 本地 memory（私有真相源）

每个 agent 的持久 memory 放在当前 workspace：

```
.agtalk/<agent-name>/memory/
  ├─ plan.md          ← 当前目标、计划、进度、阻塞项
  ├─ context.md       ← 公开背景、约束、协作注意事项
  ├─ status.json      ← 机器可读状态摘要
  └─ entries.jsonl    ← 长期记忆条目
```

- `plan.md`、`context.md`、`status.json` 是公开协作状态，其他在线 agent 可读。
- `entries.jsonl` 是长期知识沉淀，默认不跨 agent 开放。
- 这些文件只参与展示/协作，不参与认证、路由、PID 校验。

#### 2.8.3 全局用户 memory（跨 workspace，预留）

- 路径：`<config_dir>/memory/`。
- 用于未来用户可写的全局记忆，跨 workspace 生效。
- 当前只建立目录与文档约定，不引入复杂写入命令。

#### 2.8.4 SQLite 在线索引（daemon 派生视图）

daemon 维护 `mem_index` 表，只记录**当前在线** agent 的公开 mem 元数据：

```
address          UUID
name             TEXT
workspace        TEXT
memory_path      TEXT
plan_updated_at  REAL
status_summary   TEXT
public_topics    TEXT
```

- agent 上线（`id join`）时，daemon 从 `.agtalk/<name>/memory/` 读取并注册/刷新该索引。
- agent 下线（`id leave`）或 session 失效、惰性清理时，daemon 移除该索引。
- 离线 agent 的长期记忆仍在文件系统，但不再进入 daemon 的在线查询结果。
- 其他 agent 通过 `GET /api/v1/mem/plan` / `mem plan status` 只能看到已注册的在线 agent 的公开 plan/context/status。

#### 2.8.5 与身份生命周期的关系

mem 索引生命周期严格跟随 mailbox 生命周期：

```
join  → 注册 mem 索引
leave → 移除 mem 索引
惰性清理 → 移除 mem 索引
```

SQLite 中的 mem 索引可随时从文件系统重建；agent 的长期记忆以 `.agtalk/<name>/memory/` 文件为准。

### 2.9 run：YAML 编排入口

`run` 是多步 agtalk 动作的轻量编排器，**CLI 本进程执行**，不经过 REST API，也不进入 daemon 核心协议。

```bash
agtalk run [file.yaml]
```

不传文件时默认读取 `.agtalk/runs/<current-agent-name>.yaml`。

约束：

- 只执行 agtalk 内部白名单动作（如 `msg.send`、`msg.read`、`mem.plan.update`、`tool.doctor` 等），`id.join`、`config.set`、`id.leave`、任意 shell 均不允许。
- 不执行任意 shell。
- 第一阶段没有变量替换：每个 step 的字段按字面量传给对应内部动作。
- 任一步失败默认停止。

示例：

```yaml
version: 1
steps:
  - action: msg.send
    to: "550e8400-e29b-41d4-a716-446655440000"
    subject: "TASK: 实现命令面重构"
    body: "请根据 docs/commands.md 实现命令面重构。"
    notify: true

  - action: msg.read
```

`run` 不是核心通信协议，而是 CLI 便利层；GUI / 浏览器扩展等薄客户端直接调用对应 REST API 或 daemon 命令，不必经过 `run`。

---

## 3. 邮箱路由模型

### 3.1 路由唯一规则

```
send(to=<address UUID>, body, ...) → 投递到该 address 的 mailbox
```

daemon 收到 UUID → 查 mailbox 表 → 往收件箱追加消息。**路由层完全不认识 name**。

### 3.2 mailbox 生命周期 = 文件夹生命周期

**单一真相来源 = 文件系统。** 所有 mailbox 同构，生命周期由 `.agtalk/<name>/` 文件夹自然决定：

- 临时用：建文件夹 → 用 → 删文件夹（自然就是"一次性"）
- 长期用：建文件夹 → 不删（自然就是"持久"）
- daemon 的 lookup 表只是文件系统状态的镜像

不存在"持久 vs 一次性"两套机制。

### 3.3 身份消除（agtalk id leave）

```
正常路径：agtalk id leave → 通知 daemon 实时剔除 lookup → 删 .agtalk/<name>/ 文件夹
异常路径：daemon 惰性清理（lookup 时发现文件夹没了自动剔除）
```

两条路径，单一真相源，状态最终一致。惰性清理是崩溃/被 kill 的安全网。

### 3.4 四个对话域统一

进入业务消息层后无特例，全是"订阅某 UUID 的客户端"：

```
agent ↔ agent：  A id lookup(B) → msg send(uuidB) → B SSE 收
agent ↔ human：  human 是持久 mailbox（daemon 启动建），GUI 订阅 human
agent ↔ browser：见 §3.5（浏览器 = 1 持久 mailbox，插件内部按"标签↔agent"绑定表路由）
agent ↔ mobile： 见 docs/android-ble-deck.md（Android = mobile participant，BLE 只做 transport）
```

human 不再是硬编码的特殊行，browser/mobile 不再是特殊参与者类型——都是不同生命周期和不同 transport 的 mailbox。

### 3.5 浏览器扩展架构（1 浏览器 = 1 mailbox + 标签绑定表）

**核心模型**：整个浏览器插件 = **1 个持久 mailbox**（如 `name="browser-<short>"`，address 是 UUID）。agtalk 侧把浏览器当作一个普通 agent，零特殊化。多 AI 标签同开的需求由**插件内部的绑定表**解决，不污染 agtalk 协议。

**当前实现阶段**：已完成浏览器 mailbox 的创建（`/api/v1/browser/join`）、token 认证（`X-AgTalk-Browser-Token`）、SSE 订阅（`/api/v1/events`）和基础 popup/background；标签绑定表、content script 注入、AI 回复捕获为下一阶段，接口已预留。

**典型场景**：agent（kimi code / claude code / codex / zcode 等任意 agtalk agent）与浏览器里某个网页 AI（ChatGPT / Claude / ...）双向对话。

**绑定表**（插件维护，存 `chrome.storage.local`）：

```
AI 标签页 (tabId + 站点类型)  ↔  绑定的 agent address(UUID)
─────────────────────────────────────────────────────────
chatgpt 标签 (tabId=42)        ↔  agent-A 的 UUID
claude 标签  (tabId=87)        ↔  agent-B 的 UUID
```

- 绑定关系**严格 1:1**：一个 AI 标签绑一个 agent address（避免回复路由歧义）；一个 agent address 同时只绑一个 AI 标签。
- 绑定由**插件 popup 手动配**：用户在 popup 里选"哪个 agent 绑定哪个 AI 标签"。
- 绑定的 value 是 agent 的 **address(UUID)**，与具体是哪种 agent 无关（kimi/claude/codex/zcode 都行）。

**路由（复用 agtalk UUID 机制，无新概念）**：

```
入站（agtalk → 浏览器）：
  agent 发 send(browser-address, body)
  → daemon 推送给插件的 SSE 连接（带 from_address = agent 的 UUID）
  → 插件查绑定表：from_address → tabId
  → content script 注入该 AI 标签的输入框

出站（浏览器 → agtalk）：
  AI 标签产生回复 → content script 捕获
  → 插件查绑定表：tabId → 绑定的 agent address
  → send(该 agent address, 回复正文)
```

**路由键 = from_address（发送方 UUID）↔ tabId**。完美复用 agtalk 已有的 UUID 机制，不引入新概念。

**浏览器 mailbox 生命周期**：
- 插件首次连接 daemon 时创建（如 `agtalk id join browser --intro "浏览器桥接"`），address 存 chrome.storage，跨会话复用。
- 插件卸载/重装时，旧 mailbox 由 daemon 惰性清理（与普通 agent 一致）。

**未绑定 agent 发消息来的处理**：
- 插件收到消息，查绑定表发现 from_address 没绑任何标签 → **忽略注入**，并通过 agtalk 回复一条错误提示给该 agent："你尚未绑定到任何 AI 标签，请在浏览器插件 popup 里配置绑定。"
- 这样 agent 知道要提示用户去配绑定。

**一个 AI 标签绑多个 agent？** 不允许。绑定严格 1:1，因为一个标签的回复只能路由回一个 agent，多了就歧义。如果用户想让两个 agent 都和同一个 chatgpt 对话，应开两个 chatgpt 标签分别绑定。

---

## 4. 推送机制（SSE，唯一形态）

### 4.1 设计原则：推送用 SSE，但 agent 接收消息有两条路径

daemon → agent 的推送底层是 **SSE（Server-Sent Events）**，没有长轮询、没有短轮询。但 agent **接收消息**有两条路径，agtalk 都支持，agent 按自身能力自选：

**路径 1：CLI pull（所有 CLI agent 通用）**
- agent 反复调 `agtalk msg inbox` / `agtalk msg read`，每次秒级返回，agent 自己控制查询节奏。
- 零依赖——任何能执行 shell 命令的 agent（Kimi、codex CLI 等）都能用，契合"接收→调工具→返回"的核心循环。
- 这是 agtalk-office 实战验证的方式（`msg read` 取未读消息，agent 循环调）。

**路径 2：HTTP SSE（常驻进程 + 有 HTTP 工具能力的 agent）**
- daemon 暴露 `GET /events`（127.0.0.1）SSE 端点，按自己的 UUID 过滤推送。
- **常驻进程**（GUI、浏览器扩展 background）直接 fetch 持续订阅。
- **有 HTTP 工具能力的 agent**（如 codex / claude code）有两种消费方式：
  - **官方封装 `agtalk msg wait <sent-msg-id> [--timeout] [--since]`**（推荐）：agtalk 替 agent 封装 SSE 连接 + 身份认证 + Last-Event-ID 续传 + 命中目标即退 + 超时返回。`<sent-msg-id>` 是自己刚发出去的消息 ID（`msg send` / `msg ask` 返回）。
  - **自己 curl / fetch**（想精细控制时）：fetch → 解析 SSE 流 → 命中目标后 abort，务必带超时。
- 支持 `Last-Event-ID` 断线续传。

> **`wait` vs `events`**：CLI 层提供 `agtalk msg wait`（会返回的 SSE 封装，带 `--timeout`，给 agent 等"特定消息"用），但**不提供** `agtalk events`（永不返回的长驻订阅，会占住 agent 整个 turn 被强杀）。常驻订阅由常驻进程直接 `GET /api/v1/events` 完成，不经 CLI。底层都是同一个 daemon SSE 推送通道。
>
> 参考实现见 `docs/sse-demo/`（验证 SSE 推送可行；生产需补持久化 + Last-Event-ID + UUID 过滤 + 认证）。

### 4.2 SSE 订阅模型（按 UUID）

```
常驻进程（GUI / 扩展 background）或有 HTTP 能力的 agent：
  ① 身份解析：
     - CLI/GUI：PID → agents.json → name → session.json → address(UUID)
     - 浏览器扩展：chrome.storage.local → {address, token}
  ② GET /events + 凭证 → daemon 注册为"订阅 address=<UUID>"
     - 浏览器扩展带 Headers: X-AgTalk-Address, X-AgTalk-Browser-Token
  ③ 任何 send(to=<该 UUID>) 的消息 → 推给这条 SSE 连接
```

**订阅过滤器 = agent 自己的 address UUID。** UUID 既是投递目标也是订阅键，一个概念贯穿到底。

### 4.3 daemon 内部唤醒（零轮询，事件驱动）

```
handle_send(写完一条 to=uuid 的消息):
  ├─ 消息先持久化到 mailbox 收件箱（DB）     ← 先存，不丢
  ├─ 分配单调递增 event_id
  └─ 唤醒订阅了 uuid 的 SSE 连接             ← 瞬间推送，不轮询
```

SSE 连接平时阻塞 await，只有 `handle_send` 主动唤醒才推。**有内容才推，daemon 不需要"监测"。** 参考实现见 `docs/sse-demo/`。

### 4.4 抗 compact 重连 + 不丢消息（Last-Event-ID）

```
agent compact → SSE 断开（期间 daemon 又投递 51,52,53）
agent 恢复 → 重新认证 + GET /events, Header: Last-Event-ID: 50
daemon → 从 DB 重放该 UUID 下 event_id > 50 的消息 → 继续正常订阅
```

消息推送前先持久化（at-least-once）+ event_id 单调（DB 真实行）+ Last-Event-ID 续传 = 断线不丢。compact 压不到 DB，也压不到 HTTP 层的 Last-Event-ID。

---

## 5. 打扰层（notify）：解决"agent 会偷懒"

notify 插件机制的完整设计见 `docs/notify-plugin.md`。本节只描述 notify 在总体架构中的定位与红线。

### 5.1 问题：pull 模型的根本局限

第 4 节的 SSE / `msg inbox` 都是 **pull 模型**——agent 必须主动连 SSE 或调 `msg read` 才能收到消息。但 agent 的核心循环是"接收用户消息 → 调工具 → 返回"，**主动查收件箱"不产生即时价值"**，agent 不会自发地在循环里加这一步（即使用 skill/AGENTS.md 要求它，它也常跳过）。

结果：消息躺在 daemon 里，agent 不来取，对话中断。这是 pull 模型的根本局限，**不能靠协议解决，只能靠"打扰"**。

### 5.2 设计：daemon 主动打扰 agent 的执行环境

agtalk 在 pull 层（SSE / `msg inbox`）之外，增加**打扰层（notify）**：daemon 有新消息时，**主动**把"有消息"这个信号推到 agent 的执行环境，让 agent 在它的核心循环里"撞见"，不得不注意。

**关键原则（轻量信号）**：notify **只推"有新消息"的信号，不推正文**。agent 看到信号后自己调 `agtalk msg inbox` / `agtalk msg read` 取正文。理由：
- 注入量小、风险低（正文里有 shell 元字符也不会被执行）。
- agent 仍有自主权（看到信号后决定何时处理）。
- 务实目标：**最大化让 agent 注意到、降低处理门槛**，而不是幻想"push 了 agent 就一定处理"——后者是 agent 行为问题，非通信协议能完全解决。

信号形式（推荐）：注入一行既提示又带取信指令的文本（参考 agtalk-office）：
```
[agtalk] 新消息来自 <from_name>，运行 `agtalk msg read` 查看
```
agent 在终端里撞见这行 → 它的核心循环把它当输入 → 自然去执行 msg read → 拿到正文。这是 agtalk-office 已验证有效的模式。

### 5.3 多通道：四种环境各自的触达方式

agent 可能跑在任何环境（终端+zellij/tmux、普通终端、GUI/IDE、后台进程），**没有任何单一通道能触达所有环境**。notify 必须多通道，按 agent 注册时声明的环境选择：

| agent 环境 | notify 通道 | 机制 | 局限 |
|---|---|---|---|
| 终端 + zellij | zellij write-chars | `zellij action write-chars` 注入到目标 pane | 依赖 ZELLIJ_SESSION_NAME/ZELLIJ_PANE_ID；agtalk-office 已实现 |
| 终端 + tmux | tmux send-keys | `tmux send-keys -t <target>` 注入 | 依赖 TMUX_PANE；agtalk-office 已实现 |
| 普通终端（无多路复用器） | **无标准方式** | — | **难点**：没有 API 往另一个终端进程的 stdin 写字。agtalk-office 对此无解（跳过）。v2 也不假装能解决，仅依赖 agent 自查（msg read）或建议用户在 zellij/tmux 里跑 agent |
| GUI / IDE | 系统通知 + Tauri 弹窗 | OS 通知中心 / Tauri window 通知 | 需要前端展示组件；适合人类 agent 或带 GUI 的 agent |
| 后台进程 | watch 文件 / HTTP 回调 | daemon 写 `<workdir>/.agtalk/notify.flag`，或 POST agent 注册的 webhook | agent 需主动监听文件/webhook |

**注册时声明环境**：agent `id join` 时通过 `--notify <channel>` 声明自己希望被打扰的通道（如 `zellij` / `tmux` / `gui` / `webhook:<url>` / `none`）。daemon 按此选择通道。未声明 = `none`（不打扰，纯 pull）。

### 5.4 notify 与 pull 的关系（互补，非替代）

```
打扰层（notify）：daemon 主动 → 信号推到 agent 环境 → agent 注意到
                                         ↓
                                    agent 主动调
                                         ↓
拉取层（pull）：   agent 主动 → msg inbox / msg read / SSE → 取到正文
```

- notify 是"敲门"，pull 是"开门取信"。两者都需要。
- notify 失败（agent 不在 zellij/tmux、或进程不在了）→ 退化为纯 pull，消息仍在 DB 不丢。
- notify 不是必须的：agent 若自觉调 msg read，可以 `id join --notify none` 完全关掉打扰。

### 5.5 安全约束（沿用 agtalk-office 的好设计）

- notify 注入终端的文本**只含信号 + 命令模板，绝不含消息正文**（防 shell 注入）。
- 外部 notify 命令插件（CommandPlugin）路径必须绝对，参数数组执行（不经 shell）。
- 注入的命令模板末尾 `agtalk msg read` 会读 stdin 等待正文——这是预期行为，但若 agent 当前 pane 在交互提示中（如 sudo 密码），该文本会被当输入。属功能固有风险，文档需说明。

---

## 6. 数据模型

```
mailbox
  address    UUID PK           ← 路由锚
  name       TEXT              ← 可读名（不唯一）
  intro      TEXT              ← 自我介绍（lookup 消歧用）
  workspace  TEXT              ← 工作区（lookup 消歧用）
  created_at REAL

messages
  id          TEXT PK
  to_address  UUID             ← 收件 mailbox（路由用）
  from_address UUID            ← 发件 mailbox
  body        TEXT
  content_type TEXT            ← text / approval_request / approval_response / ...
  reply_to_id TEXT             ← 回复链
  metadata    TEXT             ← JSON
  event_id    INTEGER          ← 单调递增，SSE Last-Event-ID 重放用
  created_at  REAL

（投递状态、附件等子表按需添加，遵循同领域就近原则）
```

状态字段（如投递状态 pending→delivered→read→done）为 TEXT，**应用层强制状态机**（同 agtalk-office 的设计）。状态转移在 daemon 的 handler 里完成，DB 不加 CHECK（SQLite ALTER TABLE 加 CHECK 困难）。

---

## 7. 技术栈

| 层 | 选型 |
|---|---|
| 语言 | Rust 2021 |
| async runtime | tokio |
| HTTP server | axum 0.7（SSE 端点 + msg send/id lookup） |
| GUI 外壳 | Tauri 2 |
| 数据库 | SQLite（rusqlite bundled） |
| CLI 参数 | clap 4 |
| 序列化 | serde / serde_json / serde_yaml |
| 前端 | Vue 3 + Vite |
| 浏览器扩展 | WXT + Vue 3 + Pinia + Tailwind |
| 错误处理 | thiserror（领域错误） |
| 日志 | tracing |
| 单二进制 | 是（agtalk argv 分派 daemon/gui/cli/popup） |

**为什么 Rust**：Tauri 绑定（GUI 必须 Rust）+ sum-type 协议（ClientMsg/ServerMsg 用 enum + match 极优雅）+ 单机本地场景性能无关紧要但类型安全重要。

---

## 8. 目录结构

```
agtalk/                             ← 本项目根
├── Cargo.toml                      ← workspace 根（仅含 src-tauri 一个 Rust member）
├── AGENTS.md                       ← 开发要求（见该文件）
├── Makefile
├── README.md
├── src/                            ← Tauri GUI 前端（Vue 3）
│   ├── App.vue
│   ├── main.ts
│   ├── views/
│   ├── lib/
│   └── styles/
├── src-tauri/                      ← 唯一 Rust crate：bin + lib + daemon 核心逻辑
│   ├── Cargo.toml                  ← 定义 bin `agtalk` + lib `agtalk_app`
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── build.rs
│   └── src/
│       ├── main.rs                 ← bin：只做 argv 分派，< 100 行
│       ├── lib.rs                  ← lib 入口：run_gui / run_popup / run_cli
│       ├── commands.rs             ← Tauri 命令（薄桥）
│       ├── proto.rs                ← ClientMsg/ServerMsg（协议内聚）
│       ├── identity/               ← 身份：mailbox、session.json、agents.json、PID
│       │   ├── mod.rs
│       │   ├── mailbox.rs
│       │   ├── session_file.rs
│       │   ├── agents_map.rs
│       │   └── tests.rs
│       ├── routing/                ← 路由：send、lookup
│       │   ├── mod.rs
│       │   ├── send.rs
│       │   ├── lookup.rs
│       │   └── tests.rs
│       ├── mem/                    ← 计划、上下文、长期记忆
│       │   ├── mod.rs
│       │   ├── plan.rs
│       │   ├── index.rs
│       │   └── tests.rs
│       ├── cli/                    ← CLI 子命令、客户端与本地 YAML runner
│       │   ├── mod.rs
│       │   ├── runner.rs
│       │   ├── context.rs
│       │   ├── output.rs
│       │   └── client/
│       ├── tool/                   ← doctor、version、path 等工具
│       │   ├── mod.rs
│       │   └── doctor.rs
│       ├── transport/              ← 推送：SSE、唤醒、可选 BLE
│       │   ├── mod.rs
│       │   ├── sse.rs              ← SSE 端点 + Last-Event-ID 重放
│       │   ├── wake.rs             ← handle_send 唤醒订阅者
│       │   ├── ble/                ← Android Deck BLE transport（feature = "ble"）
│       │   └── tests.rs
│       ├── server/                 ← HTTP/socket 入口
│       │   ├── mod.rs
│       │   ├── http.rs             ← axum routes（薄）
│       │   ├── socket.rs           ← Unix socket（如保留）
│       │   └── tests.rs
│       ├── storage/                ← DB 句柄 + 迁移（不塞业务查询）
│       │   ├── mod.rs
│       │   ├── migrate.rs
│       │   └── tests.rs
│       └── config.rs               ← AgConfig
├── extension/                      ← 浏览器扩展（WXT/Vue 3，独立）
├── android/                        ← Android Deck APK（Kotlin/Compose，待实现）
├── docs/
│   ├── design.md                   ← 本文档
│   ├── android-ble-deck.md         ← Android Deck + BLE transport 设计
│   └── sse-demo/                   ← SSE 参考实现（见其 README）
└── examples/
```

**关键设计**：
- Rust 代码集中在 `src-tauri/` 一个 crate 内，bin + lib + daemon 核心逻辑同 crate（参考 agtalk-office）。
- 领域内按 identity / routing / mem / transport / server / storage / tool 分子模块，每领域自带 tests.rs；`run` 作为 CLI-local YAML runner 放在 `cli/` 下，不进入 REST API。
- storage 模块只管 DB 句柄和迁移，业务查询分散到各领域模块（避免 god-object）。
- proto.rs 集中放 enum 定义（协议是跨模块契约，需内聚），但不放 handler。
- 文件大小目标：每个 .rs 文件 < 500 行，绝不超 800 行。

---

## 9. 已实现阶段确定的关键细节

- **传输**：HTTP-only，所有客户端（CLI / GUI / 扩展）走 `127.0.0.1:<port>`；SSE 是唯一的推送机制。
- **鉴权**：
  - CLI/GUI：HTTP header 中带上 `X-AgTalk-Address`（来自 `session.json` 的 UUID），可选 `X-AgTalk-Pid` 与 `X-AgTalk-Start-Time` 做 PID 复用防护；daemon 以文件系统（`session.json` + `agents.json`）为信任根。
  - 浏览器扩展：使用 `X-AgTalk-Address` + `X-AgTalk-Browser-Token`，token 由 `/api/v1/browser/join` 颁发并存储在 `chrome.storage.local`。
  - Android BLE：配对后使用 `device_id + mobile device token`，token hash 存在 SQLite trusted device 记录中；该例外仅限 BLE transport。
- **agents.json 写入时机**：`agtalk id join` 时由 daemon 写入，键为进程 pid，值为 `{ name, start_time }`。
- **状态机**：
  - `messages.status`: pending / delivered / read / done / dismissed
  - `messages.content_type`: text / approval_request / approval_response / system
- **human mailbox**：daemon 启动时自动创建/复用，地址写入 `system_mailboxes(role='human')`。
- **event_id**：每个 mailbox 独立单调递增，作为 SSE `Last-Event-ID` 断线重放锚点。

---

## 10. Agent-First 便利层

本节描述为 CLI agent 提供的便利性设计。这些设计不改变核心架构：路由仍只认 UUID，name 只用于本地身份选择和展示。

### 10.1 id join 幂等

`agtalk id join <name>` 是幂等操作：

- `.agtalk/<name>/session.json` 已存在 → 复用原 address，只重新绑定当前进程/会话锚点。
- session.json 不存在 → 新建 mailbox、session、agents.json 记录。
- 传入 `--intro` / `--workspace` 时更新展示元数据；未传入保留旧值。
- 若 session 存在但 DB 中 mailbox 被误标 `left_at` 或缺失，daemon 按 session 中的 address 恢复 mailbox 和 event sequence。

这样 agent 可以在每轮任务开始时安全地执行 `id join <name>`，不用担心重复创建身份。

### 10.2 身份选择器：`--as` / `AGTALK_NAME`

为支持一个工作目录下有多个 agent session，CLI 提供身份选择器：

```bash
agtalk --as <name> <cmd>
AGTALK_NAME=<name> agtalk <cmd>
```

**限制**：该选择器只用于决定读取哪个本地 `session.json`，不参与消息路由。消息路由仍由 `session.json` 中的 UUID 决定。

`Context::current()` 身份解析优先级：
1. `--as <name>`
2. `AGTALK_NAME=<name>`
3. 已注册祖先 PID（现有 PID 链）
4. 当前目录只有一个 session 时自动恢复
5. 多个 session 且无法判断 → `identity_ambiguous`

### 10.3 notify 默认 auto

`agtalk id join` 的 `--notify` 默认值为 `auto`：

- 检测到 `ZELLIJ_SESSION_NAME` + `ZELLIJ_PANE_ID` → `zellij`。
- 否则检测到 `TMUX` / `TMUX_PANE` → `tmux`。
- 否则 `none`。

用户仍可显式 `--notify none` 关闭。notify 配置随 session.json 持久化，`msg send` / `msg reply` / `msg ask` 投递消息后异步触发。notify 失败只记日志，不影响消息投递。

### 10.4 session.json 扩展字段

为支持 notify 和调试，session.json 增加以下字段（均带默认值，旧版本兼容）：

```json
{
  "command": "<注册时当前 agtalk 二进制路径>",
  "notify_channel": "zellij",
  "notify_target": {
    "type": "zellij",
    "session": "...",
    "pane": "..."
  }
}
```

- `command`：方便人类查看该身份是由哪个命令创建的。
- `notify_channel`：持久化的 notify 通道。
- `notify_target`：zellij/tmux 定位信息，用于精准注入提示。

### 10.5 `--json` 稳定输出

常用命令支持 `--json`，方便 agent 解析：

```bash
agtalk --json id show
agtalk --json id lookup
agtalk --json msg send
agtalk --json msg ask
agtalk --json msg reply
agtalk --json msg inbox
agtalk --json msg read
agtalk --json msg wait <sent-msg-id> --timeout 30
```

错误在 `--json` 模式下统一输出到 stderr：

```json
{ "type": "error", "code": "...", "message": "..." }
```

### 10.6 `msg read` 空 inbox 稳定错误

`agtalk msg read` 在 inbox 为空时返回稳定错误码 `inbox_empty`（非 0 exit），便于 agent 判断"没有新消息"而不是解析泛化的 "not found"。
