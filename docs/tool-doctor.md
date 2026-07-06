# agtalk tool doctor 设计

版本：v0.1

定位：`agtalk tool doctor` 是给 agent 使用的环境体检命令。它用于一次性定位 daemon、身份、消息、wait/SSE、notify 等关键链路哪里断了。它不是日常状态展示命令，也不替代 `daemon status`、`config`、`id show`、`msg read` 或 `wait`。

---

## 1. 目标

`tool doctor` 解决的问题是：

```text
agent 发不出消息 / 读不到消息 / wait 不返回 / notify 不触发 / 身份无法解析时，
能用一条命令知道问题在哪，以及下一步该执行什么修复命令。
```

核心目标：

1. 无身份也能运行。
2. daemon 不运行也能运行。
3. 本地检查优先，HTTP 检查作为增强。
4. 文本输出给人和 agent 都容易扫读。
5. JSON 输出稳定，方便 agent 自动判断与修复。
6. 第一版只诊断，不自动修改状态。

非目标：

1. 不自动修复。
2. 不消费消息。
3. 不标记消息已读。
4. 不替代 `daemon status`。
5. 不做 BLE / browser / GUI 的深度诊断，除非它们已启用或已有状态。

---

## 2. 命令

当前入口：

```bash
agtalk tool doctor              # 默认摘要：根因 + Actions + Context
agtalk tool doctor --debug      # 完整检查矩阵
agtalk --json tool doctor       # 稳定 JSON（含完整 checks / root_causes / actions / context）
```

暂缓以下扩展：

```bash
agtalk tool doctor --fix
agtalk tool doctor --strict
agtalk tool doctor --scope identity|daemon|notify|message|all
```

原因：doctor 是低频调试命令，命令面要克制。第一版应先把诊断质量做好。

---

## 3. 设计原则

### 3.1 无身份也能运行

`doctor` 不能依赖 `Context::current()` 成功。身份链坏掉时，doctor 仍必须能告诉 agent：

```text
identity.current warn
原因：找不到注册过的 ancestor pid / 多个 session 无法消歧 / session.json 不存在
建议：使用 AGTALK_NAME=<name> 或 agtalk --as <name>
```

### 3.2 daemon 不运行也能运行

daemon 挂掉时，doctor 仍应检查：

1. 当前二进制路径。
2. agtalk 版本。
3. config 是否能读取。
4. daemon 状态文件是否残留。
5. pid 是否仍存活。
6. 端口是否被占用。
7. `.agtalk/` 与 session 文件是否存在。

### 3.3 本地优先

检查顺序：

```text
本地文件 / 配置 / pid / DB
  ↓
daemon HTTP
  ↓
SSE / notify 运行时能力
```

HTTP 不可达不能让 doctor 整体失败，只能让相关检查项变为 `error` 或 `warn`。

### 3.4 只诊断关键路径

第一版只看 agent 实际使用 agtalk 时最关键的链路：

```text
runtime → daemon → identity → message → wait/SSE → notify
```

BLE、browser extension、GUI 不是第一版核心路径，除非配置已启用或状态文件存在，否则不显示。

---

## 4. 检查分类

## 4.1 Runtime

检查当前 CLI 自身是否可信。

检查项：

| ID | 说明 |
| --- | --- |
| `runtime.binary` | 当前执行的 agtalk 二进制路径 |
| `runtime.version` | 当前 CLI 版本 |
| `runtime.cwd` | 当前工作目录 |
| `runtime.config_dir` | 配置目录 |

价值：

```text
避免 agent 调试时误用全局旧版 agtalk。
```

示例：

```text
runtime.binary        ok     /Users/.../target/debug/agtalk
runtime.version       ok     0.1.0
runtime.cwd           ok     /Users/linxinhong/projects/agtalk
```

---

## 4.2 Daemon

检查 daemon 是否可用。

检查项：

| ID | 说明 |
| --- | --- |
| `daemon.status_file` | daemon.json 是否存在且可解析 |
| `daemon.pid` | pid 是否存活 |
| `daemon.http` | HTTP API 是否可访问 |
| `daemon.version` | daemon 版本是否与 CLI 一致 |
| `daemon.port` | 配置端口是否可用或被 daemon 占用 |

严重性：

| 场景 | 状态 |
| --- | --- |
| daemon 未运行 | `error` |
| 状态文件残留 | `warn` |
| pid 存活但 HTTP 不可达 | `error` |
| CLI 与 daemon 版本不一致 | `warn` |

修复命令：

```bash
agtalk daemon start
agtalk daemon restart
agtalk daemon stop
```

---

## 4.3 Identity

检查当前 agent 的身份链。

检查项：

| ID | 说明 |
| --- | --- |
| `identity.dot_agtalk` | 当前工作目录 `.agtalk/` 是否存在 |
| `identity.sessions` | 有多少个 session.json |
| `identity.env_name` | 是否设置 `AGTALK_NAME` |
| `identity.pid_anchor` | 能否沿父进程链找到已注册 ancestor pid |
| `identity.session_file` | 当前 session.json 是否存在且可读 |
| `identity.address` | address 是否是 UUID |
| `identity.db_mailbox` | address 是否存在于 daemon DB |
| `identity.permissions` | session.json 权限是否为 0600 |

注意：

```text
doctor 必须只读解析身份，不应为了诊断自动写 agents.json。
```

多 session 且无法消歧时：

```text
identity.current      warn   multiple sessions found
suggestion: use --as <name> or AGTALK_NAME=<name>
```

---

## 4.4 Message

检查消息读写前置条件，但不消费消息。

检查项：

| ID | 说明 |
| --- | --- |
| `message.db` | DB 是否可打开并执行 `SELECT 1` |
| `message.mailbox` | 当前 address 是否可查询 inbox |
| `message.pending` | 当前 inbox 待处理数量 |
| `message.latest_event` | 当前 address 最新 event_id |
| `message.read_ready` | `agtalk msg read` 所需身份是否完整 |

输出示例：

```text
message.inbox         ok     3 pending, latest event_id 42
message.read_ready    ok     agtalk msg read can resolve current identity
```

没有身份时：

```text
message.read_ready    warn   current identity unresolved
command: agtalk id join <name>
```

---

## 4.5 Wait / SSE

wait 和 notify 是 agtalk 的关键能力。doctor 必须重点检查 wait 的前置条件。

检查项：

| ID | 说明 |
| --- | --- |
| `wait.http` | daemon HTTP 是否可达 |
| `wait.identity` | 当前 identity 是否有效 |
| `wait.sse` | `/api/v1/events` 是否能建立 event-stream |
| `wait.replay` | 当前 latest event_id 是否可作为 replay 基线 |

约束：

1. SSE 检查必须短超时。
2. 不长时间阻塞。
3. 不等待真实业务消息。
4. 不改变消息状态。

示例：

```text
wait.sse              ok     event stream reachable
wait.replay           ok     latest event_id 42
```

---

## 4.6 Notify

notify 对 agent 有价值，但输出必须讲清楚“当前身份的通知目标”，不能输出类似 `3 channels ready` 这种不明确的信息。

检查项：

| ID | 说明 |
| --- | --- |
| `notify.channel` | 当前 session 的 notify_channel |
| `notify.target` | 当前 session 的 notify_target |
| `notify.environment` | 当前 shell 是否处于 zellij/tmux |
| `notify.plugin` | webhook/plugin 路径是否合法 |

示例：

```text
notify.session        ok     zellij target recorded
notify.target         ok     zellij session dev, pane 3
```

如果配置为 `none`：

```text
notify.session        ok     notify disabled for this agent
```

如果配置为 `auto` 但没有可用 target：

```text
notify.session        warn   notify=auto but no zellij/tmux target recorded
suggestion: rejoin inside zellij/tmux or use --notify none
command: agtalk id join <name> --notify zellij
```

---

## 5. 默认文本输出（Agent-First 摘要）

默认只展示根因、修复命令、必要上下文和 Debug 提示，减少 agent 认知噪音：

```text
  ╭●─●╮  agtalk doctor  error
  ╰─●─╯  local agent bus unavailable

  Root causes:
    error  daemon.stopped
           daemon.json 不存在，daemon 未运行
    warn   identity.dot_agtalk
           .agtalk/ 不存在

  Actions:
    1. agtalk daemon start
    2. agtalk id join <name>

  Context:
    identity:  -
    address:   -
    pending:   unavailable

  Debug:
    agtalk tool doctor --debug
    agtalk --json tool doctor
```

完整检查矩阵保留给 `--debug`：

```bash
agtalk tool doctor --debug
```

状态含义：

| 状态 | 含义 |
| --- | --- |
| `ok` | 能力正常 |
| `warn` | 可运行，但存在容易导致 agent 混淆或降级的问题 |
| `error` | 关键链路不可用 |
| `skip` | 前置条件缺失，无法检查 |

---

## 6. JSON 输出

JSON 必须稳定，供 agent 自动解析。

JSON 结构（稳定接口）：

```json
{
  "type": "tool_diagnosis",
  "status": "error",
  "summary": "local agent bus unavailable",
  "root_causes": [
    {
      "id": "daemon.stopped",
      "status": "error",
      "message": "daemon.json 不存在，daemon 未运行",
      "command": "agtalk daemon start"
    }
  ],
  "actions": [
    "agtalk daemon start"
  ],
  "context": {
    "identity": null,
    "address": null,
    "pending": null
  },
  "checks": [
    {
      "category": "daemon",
      "name": "daemon.status_file",
      "status": "error",
      "message": "daemon.json 不存在，daemon 未运行",
      "suggestion": "运行 `agtalk daemon start` 启动 daemon",
      "command": "agtalk daemon start",
      "details": null
    }
  ]
}
```

字段说明：

| 字段 | 说明 |
| --- | --- |
| `type` | 固定为 `tool_diagnosis` |
| `status` | 聚合状态：`ok` / `warn` / `error` |
| `summary` | 一句话结论 |
| `root_causes` | 聚合后的根因列表（默认文本同此） |
| `actions` | 建议执行的修复命令 |
| `context` | 当前 identity / address / pending 摘要 |
| `checks` | 完整检查矩阵（--debug 与 --json 均包含） |

聚合规则：

```text
任一 error → error
否则任一 warn → warn
否则 ok
```

根因去重规则：

```text
- daemon.status_file error → daemon.stopped
- identity.db_mailbox error + message.mailbox error → identity.stale_mailbox
- daemon stopped 后派生的 daemon.http / wait.sse / daemon.version 不进入 root_causes
```

退出码建议：

| 聚合状态 | 退出码 |
| --- | --- |
| `ok` | 0 |
| `warn` | 0 |
| `error` | 1 |

原因：`warn` 代表可运行但需要注意，不应默认打断 agent 工作流。

---

## 7. 实现边界

推荐模块：

```text
src-tauri/src/tool/
  mod.rs
  doctor.rs
```

CLI：

```text
src-tauri/src/cli/client/tool.rs
```

要求：

```text
agtalk tool doctor
```

必须本地运行，不要求 `Context::current()`。

server handler：

```text
src-tauri/src/server/handlers/tool.rs
```

只做薄转发：

```text
handle_doctor() -> tool::doctor::run()
```

daemon API 可保留：

```text
POST /api/v1/tool/doctor
```

但 CLI doctor 不能只依赖 daemon API，因为 daemon 坏了时仍要能诊断。

---

## 8. 第一版范围

第一版实现：

1. runtime binary/version/cwd/config_dir。
2. config parse/path/http_port。
3. daemon status file + pid + HTTP reachable。
4. `.agtalk/` 与 session 扫描。
5. current identity 只读解析。
6. DB open + `SELECT 1`。
7. inbox pending count，不消费消息。
8. SSE 短超时连接检查。
9. notify session target 检查。

第一版暂缓：

1. 自动修复。
2. BLE 深度检查。
3. browser extension 深度检查。
4. GUI 配置检查。
5. 大量表格美化。
6. 复杂 scope 参数。

---

## 9. 一句话总结

```text
agtalk tool doctor 是 agent 的一键环境体检：
不依赖身份、不依赖 daemon、只诊断关键链路，
并给出稳定 JSON 与下一步可执行修复命令。
```
