# agtalk 命令参考

> 版本：v1（初版）｜ 日期：2026-07-01
> 配套：架构设计见 `docs/design.md`，开发要求见 `AGENTS.md`。
> 二进制：单一 `agtalk`，argv 分派。

---

## 设计原则（贯穿所有命令）

- **路由只认 UUID**：`send` 的 `<address>` 是 UUID。name 永远不进路由路径。
- **消歧在调用方**：用 name 找人时，`lookup` 返回带 intro/workspace 的候选列表，由调用方（agent/人）选定 UUID，再用 UUID 发消息。
- **身份载体 = 文件系统**：身份存在 `.agtalk/<name>/session.json`，认证链 `PID → agents.json → session.json → UUID`（详见 design §2）。
- **SSE 是唯一推送机制**：`wait` 命令和 daemon 的 `GET /events` HTTP 端点底层都是 SSE。
  - **agent / 人**通过 CLI 命令（`wait` 阻塞等一条、`inbox` 读快照）消费，不直接接触 SSE。
  - **常驻进程**（GUI、浏览器扩展 background）直接调 daemon 的 `GET /events` HTTP 端点持续订阅，不经过 CLI。
  - CLI 层**不暴露** `events` 长驻命令——agent 等 message 用 `wait`（带 timeout 必返回，避免被 agent 执行框架当作卡死而忽略）。

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
- 对比：`wait` 是"等未来某条"，`inbox` 是"看现在有什么"——两者正交，常驻进程可启动时 inbox 拉历史、再走 HTTP SSE 听新消息。

### detail（单条详情）

```
agtalk detail <msg-id>
```
查看单条消息详情（正文、附件、投递状态、回复链）。

---

## 阻塞等待（agent 等一条消息）

```
agtalk wait <msg-id> [--timeout <秒>]
```
阻塞等待某条消息的回复/结果，**必定返回**（等到目标消息或超时）。
- 典型场景：agent 发了审批（`human --choices`），用它阻塞等人类的 choice 回复。
- 底层是 SSE（连 daemon 的推送通道，按自己的 UUID 过滤），但 CLI 封装成"会返回的命令"——收到目标消息（reply_to 指向 msg-id）就 exit 0，超时则 exit 非 0。
- **为什么不是 `events`**：`events` 是永不返回的长驻订阅，agent 执行框架可能把它当卡死而 kill/忽略。`wait` 有明确返回码，agent 一定处理（成功或超时）。
- 超时默认值与上限：实现阶段定（参考 agtalk-office 的 default_timeout=300s）。
- compact 后：重新执行身份认证链 + 重连 wait 即恢复，无需记忆任何凭据。

> **常驻进程**（GUI、浏览器扩展 background）不使用 `wait`，而是直接调 daemon 的 `GET /events` HTTP SSE 端点持续订阅。该端点是 daemon 内部的，不作为 CLI 命令暴露。

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
