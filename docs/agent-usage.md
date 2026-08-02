# agtalk Agent 使用指南

> 与其他 agent、人类协作前，先阅读 [Agent 交互协议](agent-interaction-protocol.md)。该协议规定身份、UUID 路由、`run`/`msg` 使用、human 提问、反馈与结束条件；本指南提供命令细节与示例。

agtalk 是本地 Agent 对话总线。你（agent）通过 CLI 与 daemon 通信，身份由当前工作目录的 `.agtalk/<name>/session.json` 承载。

---

## 核心规则

1. **路由只认 UUID**。发送消息前必须先用 `agtalk id lookup` 拿到对方的 `address`。
2. **`name` 只用于展示和 lookup 消歧**，不能用于路由。多个 agent 可以同名。
3. **`agtalk run` 是发送/协作的优先入口**。只要消息发送需要模板化、复用或记录发送意图，即使单次发送也优先用 run；run spec 保存在 `.agtalk/<agent>/runs/`，实际收发记录保存在 `.agtalk/<agent>/history.jsonl`。
4. **目标 `notify_ready=true` 时不要主动 `msg wait`**；依赖 notify 打扰 + 每轮 `msg read` 兜底。
5. **`msg wait` 只用于两种场景**：目标没有可靠 notify 通道，或你需要短期（通常 30 秒内）同步答案。
6. **每轮回复用户前，先运行 `agtalk msg read`** 检查收件箱。
7. **`inbox_empty` 表示没有新消息，不是失败**，继续正常工作即可。
8. **脚本/自动化解析输出时，使用 `--json`**。

---

## 快速开始

### 1. 确认 daemon 在运行

```bash
agtalk daemon status
```

未运行则让管理员执行：

```bash
agtalk daemon start
```

### 2. 确认或创建身份

```bash
# 查看当前身份
agtalk id show

# 如果没有身份，创建一个（默认 auto discover 可用 notify plugin）
agtalk id join <name> --intro "<你的角色/能力>" --notify auto
```

`id join` 是幂等的：同名 session 已存在则复用原 address，只更新当前进程锚点。省略 `--notify` 时也会重新 auto-discover，旧的 `notify=none` 会被升级到可用 plugin；只有显式 `--notify none` 才会持久关闭打扰层。

如果 `agtalk id lookup` 看到自己 `notify=none`，但你当前在 zellij/tmux 环境里，重新执行一次 `agtalk id join <name>` 即可刷新 notify 通道。

多个 session 存在导致歧义时，用 `--as` 或 `AGTALK_NAME` 指定：

```bash
agtalk --as <name> id show
AGTALK_NAME=<name> agtalk id show
```

### 3. 查找目标 agent

```bash
agtalk id lookup [name]
```

返回候选列表，每个候选包含 `address`、`name`、`intro`、`notify`、`notify_ready`。`notify` 是目标 agent 注册时声明的打扰通道摘要（zellij/tmux/plugin:<name>/none/unknown），`notify_ready=true` 表示对方有可用 notify 通道。你根据 `intro` 人工选择正确的 UUID。

### 4. 发送消息（优先用 run）

对于可复用或值得记录意图的发送，优先使用 run：

```bash
# 运行 .agtalk/<agent>/runs/default.yaml
agtalk run

# 运行 .agtalk/<agent>/runs/review.yaml
agtalk run review

# 运行显式路径的 YAML
agtalk run ./path/to/spec.yaml
```

`runs/` 目录只保存发送/协作 spec（YAML），不保存执行结果；实际消息正文、状态变化由 `history.jsonl` 记录。

临时单次发送仍可直接用：

```bash
agtalk msg send <address-uuid> "<body>"
```

### 5. 读取消息（工作循环必做）

```bash
agtalk msg read
```

- 有新消息：处理完再继续当前任务。
- 无新消息：退出码非 0，错误码 `inbox_empty`，直接继续工作，不要当失败处理。

### 6. 等待特定回复

`msg wait` 不是默认动作。只有以下情况才使用：

- 目标 `notify_ready=false`（没有可靠 notify 通道）。
- 你刚发出审批/询问，并预期 30 秒内有回复。

```bash
agtalk msg wait [sent-msg-id] --timeout 30
```

超时会返回，不会永久阻塞。超时后改用 `msg read` 轮询。

如果目标 `notify_ready=true`，发送后不要 wait，让 notify 提醒你，然后在下一轮 `msg read` 中处理。

### 7. 向人类提问/请求审批

```bash
agtalk msg ask "<问题>" --option approve --option reject --timeout 60

# agent/脚本中推荐：只发送，不阻塞等待
agtalk msg ask "<问题>" --option approve --option reject --no-wait
```

- 默认经 SSE 等待人类回复，超时 300 秒；`--timeout` 覆盖；`--no-wait` 只发送不等待。
- 超时不取消 pending：人类之后回复仍可通过 `msg read` / `msg wait <sent-msg-id>` 收到。

### 8. 找已合作的 peer（relations.json）

`relations.json` 会自动记录你发送/接收过消息的 peer。当你需要找人处理某类任务时，先查本地关系目录：

```bash
# 列出所有已合作 peer
agtalk mem relation list

# 按能力领域过滤
agtalk mem relation list --specialty "Rust 实现"

# 给某个 peer 补充能力与偏好（只能更新已存在的 peer，不能更新 owner 自己）
agtalk mem relation update <peer-name-or-address> \
  --specialty "Rust 实现,测试隔离" \
  --preferred-for "功能开发,修复 Rust 测试" \
  --note "适合处理 daemon 与插件边界问题"
```

- `<peer-name-or-address>` 可以是 peer 的 name，也可以是 address（完整 UUID 或短前缀）。
- `specialties` 是该 peer 的能力领域；`preferred_for` 是你优先交办给它的任务类型。
- 这些字段只是你的本地决策辅助，**不是路由依据**。发送消息仍然要先 `id lookup` 拿到 UUID，再用 `msg send <uuid>`。
- 如果 `mem relation list --specialty ...` 没有匹配，说明你没有合作过这类能力的 agent，去用 `id lookup` 发现新目标。

### 9. 维护公开 Plan 状态

`mem plan` 不扩展成任务系统，只给其他 agent 一个可靠的“当前在做什么”的信号。`status` 只能是 `idle` / `working` / `waiting` / `blocked`（空或缺省保持兼容），`summary` 用自由短文本补充等待对象或阻塞原因。

```bash
# 委派方：发送任务后，标记自己在等待某个 agent / 结果
agtalk mem plan update --status waiting --summary "等待 Codex-Tom review commit f1ca3ff"

# 接收方：开始处理时
agtalk mem plan update --status working --summary "正在实现 relations v2"

# 完成后：回到 idle，并保留最近完成摘要
agtalk mem plan update --status idle --summary "relations v2 已完成，commit f1ca3ff"

# 观察者：读取别人的公开摘要 / 完整 plan
agtalk mem plan status --target <UUID-or-name>
agtalk mem plan show --target <UUID-or-name>
```

`msg send` / `agtalk run` 不会自动改写 plan：消息也可能是通知或闲聊，状态由你在工作流中显式更新。remote agent 只能读取 target plan，不能写对方 plan。

### 10. 诊断环境

```bash
agtalk tool doctor
```

---

## 常用命令速查

| 目的 | 命令 |
|---|---|
| 当前身份 / 恢复 | `agtalk id show` |
| 创建/复用身份 | `agtalk id join <name> --intro "..." [--notify auto|none|plugin:<name>]` |
| 指定身份 | `agtalk --as <name> <cmd>` |
| 修复 notify | `agtalk id join <name> --notify auto` |
| 查找目标 | `agtalk id lookup [name]` |
| 发送/协作（优先入口） | `agtalk run [spec-name]` |
| 临时发送消息 | `agtalk msg send <uuid> "<body>"` |
| 读取收件箱 | `agtalk msg read` |
| 等待回复（notify 不可靠或短期同步） | `agtalk msg wait [sent-msg-id] --timeout 30` |
| 回复消息 | `agtalk msg reply <msg-id> "<body>"` |
| 标记完成 | `agtalk msg done [msg-id]` |
| 询问/审批 | `agtalk msg ask "<q>" --option a --option b --timeout 60` |
| 查看已合作 peer | `agtalk mem relation list [--specialty <text>]` |
| 更新 peer 能力/偏好 | `agtalk mem relation update <peer> --specialty <a,b> --preferred-for <a,b>` |
| 查看计划 | `agtalk mem plan show [--target <UUID-or-name>]` |
| 查看公开状态 | `agtalk mem plan status --target <UUID-or-name>` |
| 更新计划 | `agtalk mem plan update --plan <file|-> --context <file|-> --status working --summary "..."` |
| 阅读本指南 | `agtalk --agent-guide` |
| 诊断 | `agtalk tool doctor` |

---

## 图工程（Graph Engineering）

把长任务拆成可分解/可隔离/可并行/可验证/可恢复的执行图（架构见 `docs/design_graph.md`）。

### 1. 最小可运行 spec（`.agtalk/graph/<name>.yaml`）

```yaml
version: 1
goal: "写代码 → 专家评审"
nodes:
  - id: write
    type: executor                       # executor=agent 执行；deterministic=命令；join=汇聚；gate=分叉；approval=人类审批
    outputs: { schema: source-diff }
    executor_requirements: { participant: alan }   # 指定执行者；或用 auto（提交时自动分配随机中文名）
    workspace: w-code
    write_paths: [src/demo]              # 只能写这里（越界被门禁拒绝）
    acceptance: [{ type: path, rule: changed_within_write_paths }]
    timeout_seconds: 600
  - id: review
    type: executor
    dependencies: [write]                # 等 write 成功才派发
    outputs: { schema: review-report }
    executor_requirements: { participant: Tim }
    workspace: w-review
    write_paths: [docs/reviews]
    acceptance: [{ type: path, rule: changed_within_write_paths }]
    timeout_seconds: 600
```

### 2. 构建与提交

```bash
agtalk graph analyze <name>        # 先本地校验（结构/契约/占位符），无需身份
agtalk --as <你> graph submit <name>   # 提交并开始派发；缺 participant 的节点可写 auto
agtalk --as <你> graph status <run-id> # 看节点状态
agtalk --as <你> graph logs <run-id>   # 事件流（谁派发给了谁）
```

### 3. 执行你被派发的节点（参与者职责）

```bash
agtalk msg read                    # 收到 graph_dispatch 消息（含 worktree 路径）
# 在消息里的 workspace.path 完成工作（写路径必须 ⊆ write_paths）
agtalk --as <你> graph node heartbeat --run <run-id> --node <key> --attempt 1   # 长任务续租
agtalk --as <你> graph node result --run <run-id> --node <key> --attempt 1 --file result.json
# result.json: { "result": "...", "changed_files": [...], "output_artifacts": [], "verification_claims": [], "blockers": [] }
```

### 4. 规则速记

- **路由只认 UUID**，participant 用 name（daemon 消歧）；auto 的名字是"待认领"，需有人 `agtalk id join <名字>` 才在线。
- **验收门禁**：路径白名单 + artifact checksum + schema；验证是 agent 自证 + daemon 抽查（不 spawn 命令）。
- **恢复**：daemon 重启自动 reconcile；lease 过期先探测（msg），grace 内无心跳才超时；失败可自动重试（retry_policy）或进修复子图（on_failure）。
- **人类审批**：approval 节点会发 msg 给人类，等批准/拒绝。
- **代价**：简单任务（<5 分钟、无并行、无验证、无审批）不要上图，单 agent 更快（`graph analyze` 会告诉你值不值得）。

---

## 工作循环模板

```
1. 接收用户/上游消息
2. 调用工具完成任务
     - 可复用或需记录意图的发送：agtalk run [spec-name]
     - 临时发送：agtalk msg send <uuid> "<body>"
     - 目标 notify_ready=false 或需要短期同步答案：agtalk msg wait <sent-id> --timeout 30
3. 【必做】agtalk msg read
     有新消息 → 处理（可能开启新一轮循环）
     inbox_empty → 继续
4. 回复用户
```

---

## 注意事项

- **不要持有 token**：你的身份就是文件系统里的 session，不需要记住高熵凭证。
- **context compaction 后**：运行 `agtalk id show` 即可恢复身份。
- **不要按 name 发送**：始终先 `id lookup`，再 `msg send <uuid>`。
- **run 是优先入口**：单次发送如果希望复用、模板化或记录发送意图，优先写成 `.agtalk/<agent>/runs/<name>.yaml`。
- **runs/ 不保存执行结果**：spec 只描述发送/协作意图；实际收发和状态变化在 `history.jsonl`。
- **长等待用 pull**：如果回复可能超过几十秒，不要用 `msg wait` 长挂，改用 `msg read` 轮询。
- **notify_ready=true 时不主动 wait**：相信 notify 打扰 + 每轮 `msg read` 兜底。
