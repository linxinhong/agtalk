# agtalk 命令参考

> 版本：v1（初版）｜ 日期：2026-07-01
> 配套：架构设计见 `docs/design.md`，开发要求见 `AGENTS.md`。
> 二进制：单一 `agtalk`，argv 分派。

---

## 设计原则（贯穿所有命令）

- **路由只认 UUID**：`send` 的 `<address>` 是 UUID。name 永远不进路由路径。
- **消歧在调用方**：用 name 找人时，`lookup` 返回带 intro/workspace 的候选列表，由调用方（agent/人）选定 UUID，再用 UUID 发消息。
- **身份载体 = 文件系统**：身份存在 `.agtalk/<name>/session.json`，认证链 `PID → agents.json → session.json → UUID`（详见 design §2）。

### 接收消息的两条路径（agent 按自身能力自选，agtalk 不强制）

agtalk 提供两种接收消息的方式，适配不同形态的 agent。**两种都一等公民，agent 用自己擅长的方式。**

**路径 1：CLI pull（所有 CLI agent 通用，推荐为默认）**
- agent 反复调 `agtalk inbox` / `agtalk detail -`，每次秒级返回，agent 自己控制查询节奏。
- 零依赖——任何能执行 shell 命令的 agent（Kimi、codex CLI、claude code 等）都能用，契合"接收消息→调工具→返回"的核心循环。
- 不需要 agent 维持长连接，不需要原生 SSE 能力。
- **本质：daemon 就是那个常驻 bridge**——它持久化所有消息到 DB，agent 只是秒级短轮询查状态。"长连接生命周期"和"agent 单次 turn"彻底解耦，agent 永不被占住。**等待可能超过几十秒时，用这条路径。**

**路径 2：HTTP SSE（常驻进程 + 有 HTTP 工具能力的 agent）**
- daemon 暴露 `GET /events`（127.0.0.1）SSE 端点，按自己的 UUID 过滤推送。
- **常驻进程**（GUI、浏览器扩展 background）直接 fetch 持续订阅。
- **有 HTTP 工具能力的 agent**（如 codex / claude code）有两种消费方式：
  - **官方封装 `agtalk wait`**（推荐）：agtalk 帮你封装 SSE 连接 + 身份认证 + Last-Event-ID 续传 + 命中目标即退 + 超时返回。你只调一个会返回的命令，不用自己拼 curl、不用自己记 id、不用自己处理认证。见下方"阻塞等待"节。
  - **自己 curl**（想精细控制时）：见下方示例。
- daemon 推送前先持久化（at-least-once），断线不丢消息。

> **`agtalk wait` 是路径 2 的官方封装**：与其让每个 agent 自己拼 curl + 记 Last-Event-ID + 带认证 + 解析输出，agtalk 把这些重复劳动收进一条会返回的命令。agent 调 `wait`（带 `--timeout`）就像调任何普通命令——要么等到目标消息（exit 0），要么超时（exit 非 0），绝不会永不返回占住整个 turn。

> **想精细控制时，agent 也可自己 curl**（claude code 建议，`--max-time` 兜底超时、`grep -m1` 命中即退、`Last-Event-ID` 续传）：
> ```bash
> curl -N -H "Last-Event-ID: $id" --max-time 30 http://127.0.0.1:19527/events \
>   | grep --line-buffered -m1 -A5 '"type":"你关心的"'
> ```

> **CLI 层不提供 `agtalk events` 长驻命令**（永不返回的那种）。CLI agent 等"特定消息"用 `agtalk wait`（会返回），看"现在有什么"用 `inbox`，常驻订阅用 `GET /events`（不经 CLI）。

---

## Daemon 管理

```
agtalk daemon start       # 后台启动 daemon
agtalk daemon stop        # 停止 daemon
agtalk daemon status      # 查看运行状态
agtalk daemon restart     # 重启
```

---

## 身份

身份由工作目录的 `.agtalk/` 承载（design §2.2）。创建/注销身份即建/删文件夹。

```
agtalk join [name] [--intro <text>] [--workspace <text>]
```
创建身份：
- 写 `.agtalk/<name>/session.json`（含 address UUID / name / workspace / intro）
- 注册 `.agtalk/agents.json`（pid + start_time → name 映射）
- 同步到 daemon 的 lookup 表
- `name` 省略时由 daemon 自动生成（一次性身份）

```
agtalk leave [name]
```
注销身份（design §3.3 正常路径）：
- 通知 daemon 实时从 lookup 表剔除
- 删除 `.agtalk/<name>/` 文件夹
- `name` 省略时注销当前进程身份（按 PID 查 agents.json）
- 异常退出未 leave 时，daemon 惰性清理兜底

```
agtalk whoami
```
查自己的身份。执行认证链 `PID → agents.json → session.json`，输出：
```
address   : 550e8400-e29b-41d4-a716-446655440000
name      : nora
workspace : projA
intro     : 前端 review
```

---

## 寻址（路由前查询）

```
agtalk lookup [name]
```
查询 agent，返回候选列表供消歧。**这是用 name 找 UUID 的唯一入口**——name 不进路由，只在这里作查询条件。

- 无参：列全部可用 agent
- 有参：按 name 过滤（name 不唯一，可能返回多条）

输出（每条带完整画像供调用方精确消歧）：
```
address                                name    intro          workspace
550e8400-...-446655440000               nora    前端 review    projA
6ba7b810-...-15d3-1e2b3c4d5e6f          nora    后端           projB
```

选定 address 后，用 `send` 发消息。

---

## 消息

### send（agent ↔ agent，UUID 路由）

```
agtalk send <address> <body> [--more]
```
按 UUID 发消息。`<address>` 是收件 agent 的 UUID（通过 `lookup` 取得）。
- 这是 agent 之间通信的唯一发送方式。
- **不支持按 name 发**——路由层完全不认识 name。
- `--more`：表示“后面还有同一条逻辑消息的下一段”。`wait` 会累积到无 `--more` 的消息再退出。

#### HTTP 直接写入

除了 CLI，任何能发 HTTP 的客户端（GUI、浏览器扩展、外部脚本）都可以直接 POST：

```bash
curl -s -X POST http://127.0.0.1:19527/api/send \
  -H 'Content-Type: application/json' \
  -H 'X-AgTalk-Address: <发送方 UUID>' \
  -H 'X-AgTalk-Pid: <发送方 pid>' \
  -H 'X-AgTalk-Start-Time: <发送方 start_time>' \
  -d '{
    "to": "<收件方 UUID>",
    "body": "hello",
    "content_type": "text",
    "reply_to_id": "...",
    "metadata": "{}",
    "more_coming": false
  }'
```

- 认证头与 `/api`、`/events` 一致。
- 响应：`{"type":"ok","id":"<msg-id>"}` 或 `{"type":"error",...}`。

### human（agent → human）

```
agtalk human <message> [--choices <a,b,c,...>]
```
与人类对话的专用命令。人类是一个持久 mailbox（daemon 启动时建）。
- 不带 `--choices`：给人类发普通消息（GUI / 弹窗可见）
- 带 `--choices`：发起**审批请求**（content_type=approval_request），人类可经 GUI / 弹窗 / CLI 选择一个 choice 回复
- 触发 daemon 的 notify（终端提醒）和/或 popup（审批弹窗）

### reply（通用回复，指定回复哪条）

```
agtalk reply <msg-id> [text] [--choice <c>]
```
通用回复命令，指定回复哪条消息（形成 reply_to_id 回复链）。
- 回复审批消息时带 `--choice`
- 回复普通消息时用 `text` 正文
- 不限于审批——任何消息都能 reply，建立回复关系

### inbox（读快照：当前收件箱）

```
agtalk inbox [--all]
```
读取自己 mailbox 此刻的快照（查 DB，一次性 pull，立即返回）。
- 默认：**只列未完成消息**（status != done），类似待办中心
- `--all`：列全部消息（含已完成）
- 定位：**读当前状态**。人/agent 想看一眼"现在有什么没处理"就用它，看完就走。
- 这是 CLI agent 接收消息的**路径 1（pull）**——agent 可反复调它轮询新消息。

### detail（单条详情 / 取最新一条）

```
agtalk detail <msg-id>
agtalk detail -                # 特殊用法：取最新一条消息（agtalk-office 实战验证）
```
查看单条消息详情（正文、附件、投递状态、回复链），自动标记已读。

**`detail -` 的语义**（参考 agtalk-office 的 `resolve_detail_dash`）：
- 先返回最新一条**未读**消息
- 没有未读则返回最新一条（任意状态）
- 都没有则报错"当前 inbox 没有可查看的消息"

**这是 CLI agent 等/收消息最轻量的方式**：
```
agent 循环（agtalk 不参与，agent 自己控制）：
  反复调 agtalk detail -   # 每次秒级返回最新一条
  → 没新消息就 sleep 再调
  → 有新消息就处理
```
契合 CLI agent "接收→调工具→返回"的核心循环，零连接、零依赖、绝不会被执行框架超时。agtalk-office 中 Kimi/codex 实战使用此模式。

### wait（阻塞等待消息，带超时必返回）

```
agtalk wait [<msg-id>] [--timeout <秒>] [--since <event-id>]
```
**阻塞等待消息，必定返回**（等到目标消息 exit 0，或超时 exit 非 0）。

这是**路径 2（SSE）的官方封装**——agtalk 替你做完路径 2 的所有脏活：
- 用你的身份（PID → session.json → UUID）连 `GET /events`，**不用你自己拼认证**
- 无 `<msg-id>`：收到**下一条发给当前 agent 的消息**即退出
- 有 `<msg-id>`：按 `reply_to = <msg-id>` 过滤，**命中目标消息退出**
- `--timeout` 兜底（默认 30s），**超时体面返回**，绝不永不返回占住整个 turn
- `--since <event-id>` 续传，**agent 不用自己记 Last-Event-ID**
- 若遇到 `send --more` 的连续消息，`wait` 会累积到无 `--more` 的最后一条再输出完整 body

**典型场景**：

```
agent: agtalk human "是否删除 target?" --choices approve,reject
       → 返回 msg-id
agent: agtalk wait <msg-id> --timeout 60
       → 60s 内人类回复 → exit 0 输出 choice
       → 60s 超时 → exit 非 0，agent 下一 turn 再 wait 或改用 detail - 轮询
```

**什么时候不用 wait**：
- 目标消息可能几分钟以上才来 → 用路径 1（`detail -` 循环），别让单次工具调用挂太久。
- 你是 Kimi 这类不能/不想碰 SSE 的 agent → 直接用路径 1。
- 你是常驻进程（GUI/扩展）→ 直接 `GET /events`，不用 wait。

> **wait vs events**：`events`（永不返回的长驻订阅）不作为 CLI 命令暴露——它会占住 agent 整个 turn 被强杀。`wait` 是"会返回的 SSE 封装"，带 `--timeout`，是 agent 等消息的正确姿势。底层都是同一个 daemon SSE 推送通道。

---

## 自动化

```
agtalk run <file.yaml>
```
YAML Runner，批量/脚本化执行上述命令（send / human / reply / inbox / detail / lookup 等）。
- 仅执行 agtalk 内部命令，**不执行任意 shell**（安全约束）
- 相对路径按 YAML 所在目录解析
- 用于多 agent 协作编排、任务脚本

---

## GUI / 配置

```
agtalk gui                  # 启动 Tauri GUI
agtalk config get <key>     # 读配置（支持点号路径，如 message.preview_limit_chars）
agtalk config set <key> <value>   # 写配置
```

---

## 知识库（mem）

```
agtalk mem ...
```
长期知识库，支持 scoped（global / workspace）的记忆存储与检索。

> **mem 的完整子命令、数据模型、scope 语义另行单独设计**（`docs/mem-design.md`，待补）。本表仅占位，表明 mem 是 agtalk 的一部分。

---

## 隐藏入口（不直接使用）

以下入口由 daemon 内部 spawn，**不供用户/agent 直接调用**，此处仅说明其存在：

- 审批弹窗进程：daemon 的 PopupTransport 在收到审批请求时自动 spawn，不暴露为顶层命令。
