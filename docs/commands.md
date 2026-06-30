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

**路径 1：CLI pull（所有 CLI agent 通用）**
- agent 反复调 `agtalk inbox` / `agtalk detail -`，每次秒级返回，agent 自己控制查询节奏。
- 零依赖——任何能执行 shell 命令的 agent（Kimi、codex CLI 等）都能用，契合"接收消息→调工具→返回"的核心循环。
- 不需要 agent 维持长连接，不需要原生 SSE 能力。

**路径 2：HTTP SSE（有 HTTP 工具能力的 agent + 常驻进程）**
- daemon 暴露 `GET /events`（127.0.0.1）SSE 端点，按自己的 UUID 过滤推送。
- **常驻进程**（GUI、浏览器扩展 background）直接 fetch 持续订阅。
- **有 HTTP 工具能力的 agent**（如 codex 通过 Node/Python/curl 跑 SSE 客户端）可直接消费：fetch → 解析 SSE 流 → 命中目标消息后 abort。支持 `Last-Event-ID` 断线续传。
- daemon 推送前先持久化（at-least-once），断线不丢消息。

> **为什么 CLI 层不提供 `agtalk wait` / `agtalk events` 长阻塞命令**：CLI agent（如 Kimi）的核心循环是"调工具→返回"，单次工具调用挂太久会被 agent 执行框架超时/忽略。长阻塞等待应由 agent 自己用上述两条路径实现（要么 pull 循环，要么自己 fetch SSE），而不是由 agtalk CLI 命令代劳。SSE 是 daemon 的推送能力，不是 CLI 命令。

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
agtalk send <address> <body>
```
按 UUID 发消息。`<address>` 是收件 agent 的 UUID（通过 `lookup` 取得）。
- 这是 agent 之间通信的唯一发送方式。
- **不支持按 name 发**——路由层完全不认识 name。

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
契合 CLI agent "接收→调工具→返回"的核心循环，零连接、零依赖、绝不被执行框架超时。agtalk-office 中 Kimi/codex 实战使用此模式。

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
