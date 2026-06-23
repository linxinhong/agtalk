agtalk 0.1.0 
Agent 与 Agent，Agent 与人协作的本地通信工具

## 核心原则：自动已读

`read` 不是 Agent 手动维护的状态，而是由 daemon 在消息被消费时自动维护的 delivery 状态。

| 操作 | 是否 mark-read | 说明 |
|---|---|---|
| `inbox` | 是 | 查收即已读 |
| `inbox --peek` | 否 | 只看不改 |
| `chats` | 否 | 只看会话列表 |
| `detail <msg-id>` | 是 | 查看详情 |
| `attachment <att-id>` | 是 | 读取附件全文 |
| `agent -r <msg-id>` | 是 | 回复意味着已读 |
| `agent -d <msg-id>` | 是 + done | 完成意味着已读且 done |

## 配置

全局配置文件：`~/.config/agtalk/config.json`

```json
{
  "version": 1,
  "message": {
    "inbox_inline_limit_bytes": 2048,
    "preview_limit_chars": 600,
    "attachment_threshold_bytes": 8192,
    "hard_file_threshold_bytes": 262144
  },
  "storage": {
    "attachment_dir": "~/.config/agtalk/attachments"
  }
}
```

- `<= inbox_inline_limit_bytes`：`inbox` 直接展示全文。
- `inbox_inline_limit_bytes` ~ `attachment_threshold_bytes`：正文仍存 DB，`inbox` 展示 preview。
- `> attachment_threshold_bytes`：自动转附件，`messages.body` 只保存 preview。

管理配置：

```bash
agtalk config list
agtalk config get message.attachment_threshold_bytes
agtalk config set message.attachment_threshold_bytes 4096
# 修改后重启 daemon 生效
```

## 用法

```bash
agtalk <命令> [参数]
agtalk agent <消息> [选项]      最常用：给 Agent 发任务 / 回复
agtalk reply <msg-id> <choice>  回复审批请求
agtalk run [file.yaml]           从 YAML 文件执行 agtalk 命令
```

### Agent 对话

```bash
agtalk agent <消息> [选项]
  -n, --name <name>              指定目标 Agent
  -s, --subject <标题>            消息主题
  -r, --reply-to <msg-id>        回复指定消息（隐式标记已读）
  -d, --done <msg-id>            标记消息已完成（隐式标记已读 + done）
  -f, --file <path>              附件路径，可多次使用
  -i, --notify                   提醒 Agent 查收消息
      --no-enter                 提醒时不自动发送回车
      --with-mem <topic-slug>    注入指定 topic 的 Memory Pack 到消息正文
      --with-mem-limit <n>       Memory Pack 最大条数，默认 5
```

### 人类对话

```bash
agtalk human <消息> [选项]
  -q, --question <text>          提出问题，可多次出现
  -o, --option <text>            添加预定义回答选项
  -o!, --option! <text>          同 -o，并标记为推荐答案
  --single                       单选，默认多选
  --select-only                  严格选择，禁用自由文本
  --output <text|json>           输出格式，默认 text
```

人类通过 `agtalk reply <msg-id> <choice> [-r/--reason <说明>]` 回应后，Ask 发起方会立即得到结果。若 Ask 进程已退出或 daemon 重启，可稍后通过 `agtalk wait <msg-id>` 重新查询结果。

支持同时 `-r` 与 `-d`：

```bash
agtalk agent "处理完成，结论如下..." -n codex -r msg_123 -d msg_123
agtalk agent "帮我 review 这段代码" -n codex --with-mem project-setup
agtalk agent "按规范重构" -n codex --with-mem project-setup --with-mem-limit 3
```

`--with-mem` 会把指定 topic 的 Memory Pack 放在消息正文前，作为上下文注入给目标 Agent。
- topic 不存在时直接报错，消息不会发送。
- Memory Pack 为空时 CLI 会提示，并发送带 `empty="true"` 标记的结构化消息。
- 默认取 5 条 memory，可通过 `--with-mem-limit` 调整。
- archived 状态的 memory 不会进入 Memory Pack。
- 注入格式：
  ```xml
  <agtalk_memory_pack topic="project-setup">
  ...
  </agtalk_memory_pack>

  <user_message>原消息</user_message>
  ```

### 参与者

```bash
agtalk join <name>               加入本地通信网络
  --intro <text>                 Agent 自我介绍（存入 participant.intro）
  --role <role>                  Agent 角色，默认 agent
  --transport <plugin>           Agent 的通知方式，默认 terminal
  --takeover                     强制接管同 endpoint 上的旧 active session
agtalk leave                     离开本地通信网络（本地 session.json 标为 left）
  --purge                        同时删除本地 session.json 凭证
agtalk cleanup                   清理当前 workspace 已退役的 session 记录与本地凭证文件
  --dry-run                      仅列出会被清理的 participant，不删除
agtalk me                        查看 Agent 自己的信息
agtalk peers                     列出所有在线参与者
```

**session takeover 规则**：
- 同一 workspace 的同一 endpoint（zellij/tmux 的 `session:pane_id`）只能保留一个 active session。
- 新 `join` 检测到冲突时会返回错误，CLI 会提示是否接管；加 `--takeover` 可跳过确认直接接管。
- takeover 为原子操作：新 session 创建与旧 session 退役在单次事务中完成；若创建失败，旧 session 仍保持 active。
- shell / 无 endpoint 的 join 不参与冲突桶，不同 Agent 可在普通终端同时在线。
- 接管成功后，旧 session 会被标记为 `left`，其本地 `.agtalk/sessions/<name>.json` 同步失效，participant 在线状态按剩余 active session 重算。
- `leave --purge` 不依赖当前 session 仍可认证：即使 session 已被接管或 daemon 端失效，仍可删除本地 `.agtalk/sessions/<name>.json` 凭证。
- `cleanup` 仅清理当前 workspace 的 inactive session，不会扫描其他 workspace。

### 收件箱与对话

```bash
agtalk inbox                         查看待处理消息（待办中心）
  --peek                             只查看，不标记已读
  --unread                           仅显示未读消息
  --pending                          仅显示待处理消息
  --action-required                  仅显示需要回应的消息
  --all                              显示全部消息（包括已完成）
agtalk detail <msg-id>               查看消息详情（自动标记已读）
agtalk wait <msg-id>                 等待审批结果（daemon 重启后可恢复）
  --timeout <secs>                   最长等待秒数，默认 300
  --output <text|json>               输出格式，默认 text
agtalk attachment <att-id>           查看附件全文（自动标记已读）
agtalk chats                         查看对话列表
agtalk config <get|set|list> [key] [value]  管理全局配置
agtalk daemon <start|stop|restart|status>   管理后台 daemon
```

### 长期知识库 (mem)

以下命令优先展示长参数；短参数为快捷方式，各子命令中含义不同：
- `-t` 在 `topic add/update` 中表示 `--title`，在 `mem add/promote/search` 中表示 `--type`。
- `-s` 在 `topic add/update` 和 `mem add/promote/update` 中表示 `--summary`，在 `mem search` 中表示 `--scope`。
- `-p` 在 `topic add/update` 中表示 `--priority`，在 `mem add/promote/search` 中表示 `--topic`。

```bash
agtalk mem topic add <slug> --title <title> [--summary <summary>] [--alias <alias>]... [--priority <1-5>]
agtalk mem topic list [--all]
agtalk mem topic show <slug>
agtalk mem topic update <slug> [--title <title>] [--summary <summary>] [--alias <alias>]... [--priority <1-5>] [--archive]

agtalk mem add <content> --type <type> --title <title> --confidence <low|medium|high>
  [--summary <summary>] [--topic <topic>]... [--tags <tags>] [--importance <1-5>] [--scope <global|workspace|session>]
agtalk mem show <mem-id>
agtalk mem update <mem-id> [--content <content>] [--type <type>] [--title <title>] [--summary <summary>]
  [--topic <topic>]... [--tags <tags>] [--importance <1-5>] [--status <status>]
agtalk mem archive <mem-id>
agtalk mem promote <source-ref> [--source-type <message|artifact>] --type <type> --title <title> --confidence <low|medium|high>
  [--summary <summary>] [--topic <topic>]... [--tags <tags>] [--importance <1-5>]
agtalk mem search <query> [--topic <topic>]... [--type <type>] [--scope <global|workspace|session>] [--limit <limit>]
agtalk mem pack <topic-slug> [--limit <limit>]
```

`<mem-id>` 支持完整 UUID，也支持至少 4 位的前缀短 ID。短 ID 必须唯一匹配，否则会报错。

```bash
agtalk mem show 3f117a30
agtalk mem archive 3f117a
```

说明：
- `slug` 是 topic 的 URL 友好标识，创建后不可修改；重复创建会报错，不会静默覆盖。
- `type`（item_type）建议值：`fact`（事实）、`decision`（设计决策）、`rule`（规则/约束）、`procedure`（操作流程）、`issue`（问题/待解决事项）、`snippet`（命令/代码片段）、`preference`（偏好）、`summary`（会话/阶段总结）、`note`（普通笔记）、`context`（背景上下文）。
- `confidence` 可选值：`low`、`medium`、`high`，默认 `medium`。
- `importance` 为 1-5 的整数，默认 3。
- `scope` 可选值：`global`、`workspace`、`session`，默认 `workspace`。
- `workspace` 绑定当前工作区；`session` 绑定当前会话；`global` 为全局记忆。
- `mem add` 的 `<content>` 为正文字符串（位置参数）。
- `mem add` 指定的 topic 不存在时会直接报错，不会自动创建，避免 topic 被拼写错误污染。
- `mem search` 基于 FTS5 全文索引；当前阶段对中文分词支持有限，建议用空格/英文关键词搜索。
- `search` 与 `pack` 默认不包含 `archived` 状态的 memory。

`inbox` 返回结构示例：

```json
{
  "id": "msg_123",
  "from": { "id": "codex", "name": "codex", "type": "agent", "intro": "coding" },
  "subject": "storage.rs 重构 review",
  "content": { "mode": "preview", "body": "...", "truncated": true, "size": 12480 },
  "attachments": [
    { "id": "att_001", "role": "full_body", "filename": "...", "content_type": "text/markdown", "size": 12480 }
  ],
  "delivery": { "status": "read", "read_at": "..." },
  "actions": ["detail", "reply", "done", "attachment"],
  "action_required": false,
  "priority": "normal",
  "kind": "message"
}
```

### YAML Runner

```bash
agtalk run [file.yaml]
```

`run` 读取 YAML 文件并执行等价 agtalk 命令。Runner 只执行 agtalk 内部命令，不执行任意 shell。YAML 中的相对路径按 YAML 文件所在目录解析。省略 `file.yaml` 时，默认读取 `.agtalk/runs/<当前agent-name>.yaml`。

**授权与路径建议**：如果你运行在需要沙箱授权的环境（或想固定工作流入口），建议把复杂指令始终写入同一个文件，例如 `.agtalk/runs/<当前agent-name>.yaml`。每次只需覆盖该文件再执行 `agtalk run`，路径不变，方便一次性授权；多 active session 时可用 `AGTALK_NAME=<agent-name> agtalk run` 指定身份。

支持的顶层协议：

```yaml
version: 1
command: agent | human | reply | wait | inbox | detail | attachment | chats | peers | me | mem
```

`version` 必须为 `1`，未知 `command` 会报错。

#### agent

```yaml
version: 1
command: agent
name: kimi-coder-Kimi
subject: "TASK: 实现功能"
message: "请阅读附件并实现。"
reply_to: null
done: null
notify: true
no_enter: false
files:
  - .agtalk/kimi-handoff.md
```

字段映射：
- `name` -> `-n/--name`
- `subject` -> `-s/--subject`
- `message` -> 正文
- `reply_to` -> `-r/--reply-to`
- `done` -> `-d/--done`
- `notify` -> `-i/--notify`
- `no_enter` -> `--no-enter`
- `files` -> 多个 `-f`（相对路径按 YAML 文件目录解析）
- `with_mem` -> `--with-mem`（注入指定 topic 的 Memory Pack）
- `with_mem_limit` -> `--with-mem-limit`（Memory Pack 最大条数，默认 5）

运行时校验：
- `done` 为空时，`name` 与 `message` 均必填。
- 仅当 `done` 有值时可省略 `name` 与 `message`（对应 `agtalk agent -d <msg-id>` 标记完成）。

#### human

```yaml
version: 1
command: human
message: "部署前确认"
single: true
select_only: true
output: json
questions:
  - text: "是否继续部署？"
    options:
      - text: "继续"
        recommended: true
      - text: "停止"
```

字段映射：
- `message`：共享描述；`questions` 为空时提升为唯一问题
- `single` -> `--single`
- `select_only` -> `--select-only`
- `output` -> `text | json`，默认 `text`
- `questions[].text` -> `-q`
- `questions[].options[].text` -> `-o`
- `recommended: true` -> `-o!`

运行时校验：
- `select_only: true` 时，每个 `question` 必须至少配置一个 `option`。

#### reply / wait / detail / attachment / chats / peers / me

```yaml
version: 1
command: reply
msg_id: 12345678
choice: "允许"
reason: "已确认"
```

```yaml
version: 1
command: wait
msg_id: 12345678
timeout: 60
output: json
```

```yaml
version: 1
command: inbox
status: action_required   # unread | pending | action_required | all
limit: 50
peek: true
```

```yaml
version: 1
command: detail
msg_id: 12345678
```

```yaml
version: 1
command: attachment
attachment_id: abcdef12
```

```yaml
version: 1
command: chats
```

```yaml
version: 1
command: peers
verbose: false
```

```yaml
version: 1
command: me
```

#### mem

```yaml
version: 1
command: mem
mem_command: topic_add
slug: project-setup
title: "项目环境配置"
summary: "开发环境、依赖与构建命令"
aliases:
  - setup
priority: 4
```

```yaml
version: 1
command: mem
mem_command: add
content: "使用 pnpm + vite；构建命令 pnpm build"
title: "构建方式"
type: fact
topics:
  - project-setup
confidence: high
importance: 3
tags: "build,frontend"
```

```yaml
version: 1
command: mem
mem_command: search
query: "error handling"
topics:
  - project-setup
limit: 10
```

```yaml
version: 1
command: mem
mem_command: pack
topic_slug: project-setup
limit: 5
```

`mem_command` 可选值：
`topic_add`、`topic_list`、`topic_show`、`topic_update`、`add`、`show`、`update`、`archive`、`promote`、`search`、`pack`。

常用字段：
- `slug` / `title` / `summary` / `aliases` / `priority` / `archive` / `all`
- `content` / `type`（item_type） / `confidence` / `importance` / `scope` / `topics` / `tags`
- `mem_id` / `source_ref` / `source_type` / `query` / `topic_slug` / `limit` / `status`

字段说明：
- `type` 建议值：`fact`、`decision`、`rule`、`procedure`、`issue`、`snippet`、`preference`、`summary`、`note`、`context`。
- `confidence` 建议值：`low`、`medium`、`high`，默认 `medium`。
- `scope` 建议值：`global`、`workspace`、`session`，默认 `workspace`。
  `workspace` 绑定当前工作区；`session` 绑定当前会话；`global` 为全局记忆。
- `importance` 为 1-5 整数，默认 3。
- `topics` 为数组，元素为 topic slug；不存在的 topic 会报错。
- `tags` 为逗号分隔字符串。

### 环境

```bash
agtalk init                      初始化环境
agtalk settings                  打开设置界面
agtalk daemon <action>           管理后台服务：start, stop, restart, status
```

### 帮助

```bash
agtalk --help, -h                显示帮助信息
agtalk <命令> --help              子命令详细用法
agtalk --agent-help              面向 AI 的精简用法
```

## HTTP API

daemon 在 `127.0.0.1:<port>` 上提供 HTTP 接口（默认端口 `19527`，通过 `config.json` 中 `daemon.http_port` 配置）。

### 端点

```
POST http://127.0.0.1:19527/api
```

所有操作通过同一端点，请求/响应均为 JSON，与 Unix socket 协议格式一致。

### 认证

需要 session 的操作通过自定义 Header 传递凭证：

```
X-Agtalk-Session-Id: <session_id>
X-Agtalk-Token: <token>
```

免认证操作：`Ping`、`Auth`、`Join`、`Attach`。

### 请求格式

`ClientMsg` 使用内部标签（`type` 字段 + snake_case 变体名），与 Unix socket 协议一致：

```http
POST /api HTTP/1.1
Host: 127.0.0.1:19527
Content-Type: application/json
X-Agtalk-Session-Id: 550e8400-e29b-41d4-a716-446655440000
X-Agtalk-Token: a1b2c3d4e5f6...

{
  "type": "send",
  "to": "kimi-coder-Quinn",
  "body": "请 review PR #42",
  "content_type": "text",
  "notify": true
}
```

### 响应格式

所有响应返回 `ServerMsg` JSON：

```json
{
  "type": "ok",
  "data": { ... }
}
```

```json
{
  "type": "error",
  "code": "session_inactive",
  "message": "当前 session 已被退役或接管"
}
```

### 操作速查

请求 body 中 `type` 字段为 `ClientMsg` 枚举变体的 snake_case 名。

| type | 需认证 | 说明 |
|---|---|---|
| `ping` | 否 | 心跳检测 |
| `auth` | 否 | 验证已有 session |
| `join` | 否 | 加入 workspace / 注册 |
| `attach` | 否 | 接管已有 peer |
| `send` | 是 | 发送消息 |
| `inbox` | 是 | 查收件箱 |
| `done` | 是 | 标记完成 |
| `detail` | 是 | 查看消息详情 |
| `attachment` | 是 | 读取附件全文 |
| `list_participants` | 是 | 列出参与者 |
| `list_conversations` | 是 | 列出对话 |
| `get_messages` | 是 | 获取对话消息 |
| `get_message` | 是 | 获取单条消息（审批弹窗用） |
| `read` | 是 | 标记已读 |
| `ask` | 是 | 发起审批请求（阻塞） |
| `reply` | 是 | 回复审批 |
| `wait` | 是 | 等待审批结果（长轮询） |
| `who_am_i` | 是 | 查询当前身份 |
| `create_conversation` | 是 | 创建对话 |
| `register` | 是 | 注册参与者 |
| `unregister` | 是 | 注销参与者 |
| `leave` | 是 | 离开当前 session |
| `cleanup` | 是 | 清理 inactive session |

### curl 示例

**Ping（无需认证）**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -d '{"type":"ping"}'
# → {"type":"ok","data":{"pong":true}}
```

**Join 注册新 Agent**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -d '{
    "type":"join",
    "workspace_root":"/path/to/project",
    "workspace_name":"my-project",
    "name":"claude-coder-Alex",
    "role":"coder",
    "intro":"编程专家",
    "transport":"terminal",
    "takeover":false
  }'
# → {"type":"ok","data":{"session_id":"...","token":"...","workspace_id":"..."}}
```

**Attach 接管已有 peer**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -d '{
    "type":"attach",
    "workspace_root":"/path/to/project",
    "workspace_name":"my-project",
    "name":"kimi-coder-Quinn",
    "takeover":true
  }'
```

**Send 发送消息**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: 550e8400-..." \
  -H "X-Agtalk-Token: a1b2c3d4..." \
  -d '{
    "type":"send",
    "to":"kimi-coder-Quinn",
    "body":"请 review PR #42",
    "notify":true
  }'
```

**Inbox 查收件箱**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: ..." \
  -H "X-Agtalk-Token: ..." \
  -d '{
    "type":"inbox",
    "participant":"claude-coder-Alex",
    "status":"pending",
    "limit":30,
    "peek":false
  }'
```

**Ask 发起审批（阻塞等待人类回复）**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: ..." \
  -H "X-Agtalk-Token: ..." \
  -d '{
    "type":"ask",
    "to":"human",
    "body":"要继续部署吗？",
    "choices":["继续","停止"],
    "timeout_secs":300
  }'
# 等待人类选择后返回：
# → {"type":"ask_response","msg_id":"...","choice":"继续","reason":""}
# 或弹窗关闭：
# → {"type":"ask_dismissed","msg_id":"..."}
# 或超时：
# → {"type":"ask_timeout","msg_id":"..."}
```

**Detail 查看消息详情**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: ..." \
  -H "X-Agtalk-Token: ..." \
  -d '{"type":"detail","msg_id":"87976946"}'
```

**Done 标记完成**

```bash
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: ..." \
  -H "X-Agtalk-Token: ..." \
  -d '{
    "type":"done",
    "msg_id":"87976946",
    "participant":"claude-coder-Alex"
  }'
```

### 带附件发送

```bash
ATT_B64=$(base64 -i ./report.md)
curl -s -X POST http://127.0.0.1:19527/api \
  -H "Content-Type: application/json" \
  -H "X-Agtalk-Session-Id: ..." \
  -H "X-Agtalk-Token: ..." \
  -d "{
    \"type\":\"send\",
    \"to\":\"kimi-coder-Quinn\",
    \"body\":\"分析一下这个报告\",
    \"attachments\":[{
      \"filename\":\"report.md\",
      \"content_type\":\"text/markdown\",
      \"data\":\"$ATT_B64\",
      \"role\":\"user_file\"
    }]
  }"
```

### 错误响应

```json
{
  "type": "error",
  "code": "session_inactive",
  "message": "当前 session 已被退役或接管"
}
```

| code | 说明 |
|---|---|
| `parse_error` | 请求 body 无法解析 |
| `auth_failed` | session_id 或 token 无效 |
| `session_inactive` | session 已被退役或接管 |
| `participant_not_found` | peer 不存在（attach 时） |
| `session_conflict` | 同 endpoint 已有 active session，需 takeover |
| `message_not_found` | 消息不存在 |
| `not_approval_request` | 非审批消息不能 Reply |

### 端口配置

`~/.config/agtalk/config.json`：

```json
{
  "version": 1,
  "daemon": {
    "http_port": 19527
  }
}
```

`http_port` 默认 `19527`。修改后需重启 daemon。

### 安全约束

- daemon 仅绑定 `127.0.0.1`，外部网络无法访问
- 所有需认证操作必须携带有效的 session_id + token
- token 在 Join/Attach 时由 daemon 生成，存储在本地 session 文件中

## 附件 role

| role | 含义 |
|---|---|
| `full_body` | 超长消息的完整正文（由 daemon 自动拆分） |
| `user_file` | 用户或 Agent 显式附加的文件 |
| `generated_report` | Agent 生成的报告 |
| `patch` | diff / patch 文件 |
| `log` | 命令输出日志 |
| `artifact` | 生成物：设计稿、文档、图片等 |

## 说明

- `chats` 关心“有哪些会话”，展示当前身份参与过的全部 conversation summary，包括已读、未读、已完成和历史会话。
- `inbox` 关心“我现在要处理什么”，展示当前身份作为 recipient 且尚未 done 的消息待办项，按优先级排序。默认显示所有未 done 的待处理项。
- `inbox` 默认会消费消息并 mark-read；使用 `--peek` 可只看不改状态。
