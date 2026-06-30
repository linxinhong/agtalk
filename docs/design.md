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

### 2.4 PID 复用防护

`agents.json` 存 `{pid, name, start_time}`。daemon 校验时比 `pid + 进程启动时间` 双因子。PID 被 OS 复用时 start_time 不一致，识别得出。

### 2.5 lookup 查询接口

```
GET /lookup?name=nora  （或无参列全部）
→ [{address: UUID, name: "nora", intro: "前端 review", workspace: "projA"},
   {address: UUID, name: "nora", intro: "后端",       workspace: "projB"}]
```

agent 用返回的 intro + workspace 在同名情况下精确消歧，选定 address 后再用 UUID 发消息。

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

### 3.3 身份消除（agtalk leave）

```
正常路径：agtalk leave → 通知 daemon 实时剔除 lookup → 删 .agtalk/<name>/ 文件夹
异常路径：daemon 惰性清理（lookup 时发现文件夹没了自动剔除）
```

两条路径，单一真相源，状态最终一致。惰性清理是崩溃/被 kill 的安全网。

### 3.4 三个对话域统一

无特例，全是"订阅某 UUID 的客户端"：

```
agent ↔ agent：  A lookup(B) → send(uuidB) → B SSE 收
agent ↔ human：  human 是持久 mailbox（daemon 启动建），GUI 订阅 human
agent ↔ browser：扩展持有持久 mailbox，content script 把 UUID 注入网页 AI 上下文，
                  AI 回复经扩展投递，扩展 SSE 收 daemon 推送再注入输入框
```

human 不再是硬编码的特殊行，browser 不再是 type='web' 的特殊参与者——都是不同生命周期的 mailbox。

---

## 4. 推送机制（SSE，唯一形态）

### 4.1 设计原则：推送用 SSE，但 agent 接收消息有两条路径

daemon → agent 的推送底层是 **SSE（Server-Sent Events）**，没有长轮询、没有短轮询。但 agent **接收消息**有两条路径，agtalk 都支持，agent 按自身能力自选：

**路径 1：CLI pull（所有 CLI agent 通用）**
- agent 反复调 `agtalk inbox` / `agtalk detail -`，每次秒级返回，agent 自己控制查询节奏。
- 零依赖——任何能执行 shell 命令的 agent（Kimi、codex CLI 等）都能用，契合"接收→调工具→返回"的核心循环。
- 这是 agtalk-office 实战验证的方式（`detail -` 取最新一条消息，agent 循环调）。

**路径 2：HTTP SSE（常驻进程 + 有 HTTP 工具能力的 agent）**
- daemon 暴露 `GET /events`（127.0.0.1）SSE 端点，按自己的 UUID 过滤推送。
- **常驻进程**（GUI、浏览器扩展 background）直接 fetch 持续订阅。
- **有 HTTP 工具能力的 agent**（如 codex / claude code）有两种消费方式：
  - **官方封装 `agtalk wait <msg-id> [--timeout] [--since]`**（推荐）：agtalk 替 agent 封装 SSE 连接 + 身份认证 + Last-Event-ID 续传 + 命中目标即退 + 超时返回。agent 调一个会返回的命令即可，不用自己拼 curl/记 id/带认证。
  - **自己 curl / fetch**（想精细控制时）：fetch → 解析 SSE 流 → 命中目标后 abort，务必带超时。
- 支持 `Last-Event-ID` 断线续传。

> **`wait` vs `events`**：CLI 层提供 `agtalk wait`（会返回的 SSE 封装，带 `--timeout`，给 agent 等"特定消息"用），但**不提供** `agtalk events`（永不返回的长驻订阅，会占住 agent 整个 turn 被强杀）。常驻订阅由常驻进程直接 `GET /events` 完成，不经 CLI。底层都是同一个 daemon SSE 推送通道。
>
> 参考实现见 `docs/sse-demo/`（验证 SSE 推送可行；生产需补持久化 + Last-Event-ID + UUID 过滤 + 认证）。

### 4.2 SSE 订阅模型（按 UUID）

```
常驻进程（GUI / 扩展 background）或有 HTTP 能力的 agent：
  ① 身份解析：PID → agents.json → name → session.json → address(UUID)
  ② GET /events + 凭证 → daemon 注册为"订阅 address=<UUID>"
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

## 5. 数据模型

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

## 6. 技术栈

| 层 | 选型 |
|---|---|
| 语言 | Rust 2021 |
| async runtime | tokio |
| HTTP server | axum 0.7（SSE 端点 + send/lookup） |
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

## 7. 目录结构

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
│       ├── transport/              ← 推送：SSE、唤醒
│       │   ├── mod.rs
│       │   ├── sse.rs              ← SSE 端点 + Last-Event-ID 重放
│       │   ├── wake.rs             ← handle_send 唤醒订阅者
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
├── docs/
│   ├── design.md                   ← 本文档
│   └── sse-demo/                   ← SSE 参考实现（见其 README）
└── examples/
```

**关键设计**：
- Rust 代码集中在 `src-tauri/` 一个 crate 内，bin + lib + daemon 核心逻辑同 crate（参考 agtalk-office）。
- 领域内按 identity / routing / transport / server / storage 分子模块，每领域自带 tests.rs。
- storage 模块只管 DB 句柄和迁移，业务查询分散到各领域模块（避免 god-object）。
- proto.rs 集中放 enum 定义（协议是跨模块契约，需内聚），但不放 handler。
- 文件大小目标：每个 .rs 文件 < 500 行，绝不超 800 行。

---

## 8. 待实现计划阶段确定的细节

- SSE 端点鉴权：凭证怎么从 session.json 带到 `GET /events`（header？query？）
- agents.json 写入时机：daemon spawn agent 时写，还是 agent 首次连接时写
- GUI 如何订阅多个 mailbox（inbox 视图要看所有消息）
- HTTP /events 之外是否保留 Unix socket（CLI/同机 agent 是否也走 SSE，还是 socket+SSE 并存）
- 状态机字段（投递状态、消息 content_type 枚举）的最终取值集
