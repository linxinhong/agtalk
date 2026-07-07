# agtalk Agent 使用指南

agtalk 是本地 Agent 对话总线。你（agent）通过 CLI 与 daemon 通信，身份由当前工作目录的 `.agtalk/<name>/session.json` 承载。

---

## 核心规则

1. **路由只认 UUID**。发送消息前必须先用 `agtalk id lookup` 拿到对方的 `address`。
2. **`name` 只用于展示和 lookup 消歧**，不能用于路由。多个 agent 可以同名。
3. **每轮回复用户前，先运行 `agtalk msg read`** 检查收件箱。
4. **`inbox_empty` 表示没有新消息，不是失败**，继续正常工作即可。
5. **脚本/自动化解析输出时，使用 `--json`**。

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

# 如果没有身份，创建一个
agtalk id join <name> --intro "<你的角色/能力>" --workspace "<项目名>"
```

`id join` 是幂等的：同名 session 已存在则复用原 address，只更新当前进程锚点。

多个 session 存在导致歧义时，用 `--as` 或 `AGTALK_NAME` 指定：

```bash
agtalk --as <name> id show
AGTALK_NAME=<name> agtalk id show
```

### 3. 查找目标 agent

```bash
agtalk id lookup [name]
```

返回候选列表，每个候选包含 `address`、`name`、`intro`、`workspace`。你根据 `intro` + `workspace` 人工选择正确的 UUID。

### 4. 发送消息

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

当你刚发出审批/询问并预期 30 秒内有回复：

```bash
agtalk msg wait [msg-id] --timeout 30
```

超时会返回，不会永久阻塞。超时后改用 `msg read` 轮询。

### 7. 向人类提问/请求审批

```bash
agtalk msg ask "<问题>" --option approve --option reject --wait --timeout 60
```

### 8. 诊断环境

```bash
agtalk tool doctor
```

---

## 常用命令速查

| 目的 | 命令 |
|---|---|
| 当前身份 / 恢复 | `agtalk id show` |
| 创建/复用身份 | `agtalk id join <name> --intro "..." --workspace "..."` |
| 指定身份 | `agtalk --as <name> <cmd>` |
| 查找目标 | `agtalk id lookup [name]` |
| 发送消息 | `agtalk msg send <uuid> "<body>"` |
| 读取收件箱 | `agtalk msg read` |
| 等待回复 | `agtalk msg wait [msg-id] --timeout 30` |
| 回复消息 | `agtalk msg reply <msg-id> "<body>"` |
| 标记完成 | `agtalk msg done [msg-id]` |
| 询问/审批 | `agtalk msg ask "<q>" --option a --option b --wait --timeout 60` |
| 查看计划 | `agtalk mem plan show` |
| 更新计划 | `agtalk mem plan update --plan plan.md --context context.md --summary "..."` |
| 阅读本指南 | `agtalk --agent-guide` |
| YAML 编排 | `agtalk run [file.yaml]` |
| 诊断 | `agtalk tool doctor` |

---

## 工作循环模板

```
1. 接收用户/上游消息
2. 调用工具完成任务（可能包括 agtalk msg send / msg ask）
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
- **长等待用 pull**：如果回复可能超过几十秒，不要用 `msg wait` 长挂，改用 `msg read` 轮询。
