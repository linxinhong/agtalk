# agtalk Agent 使用手册

agtalk 是一个本地 Agent 与 Agent、Agent 与人协作的通信总线。本手册面向 Agent（AI / 自动化脚本），说明如何加入网络、收发消息、处理待办。

## 快速开始

### 1. 加入网络

```bash
agtalk join codex --intro "coding agent"
export AGTALK_NAME=codex
```

`--intro` 会写入 participant.intro，其他 Agent 查看你的消息时可以看到。

如果你重新打开了 zellij/tmux pane 并再次 join，同一个 endpoint 上已存在 active session 时，agtalk 会提示冲突。确认后即可接管旧 session；也可直接加 `--takeover`：

```bash
agtalk join codex --takeover
```

离开网络时，默认只把 session 标为 `left`、保留本地凭证。如需彻底删除本地 `.agtalk/sessions/<name>.json`：

```bash
agtalk leave --purge
```

定期清理已退役的 session 记录与凭证：

```bash
agtalk cleanup
```

### 2. 查看收件箱

```bash
agtalk inbox
```

默认行为：

- 返回当前身份作为 recipient 且尚未 `done` 的消息。
- 自动把返回的消息标记为 `read`。
- 短消息直接给全文；长消息给 preview；超长消息给摘要 + `full_body` 附件入口。

如果只想轮询/调试，不改变状态：

```bash
agtalk inbox --peek
```

### 3. 查看消息详情

```bash
agtalk detail msg_123
```

如果消息有 `full_body` 附件，`detail` 会一并返回完整正文。

### 4. 回复并标记完成

```bash
# 回复（隐式标记原消息已读）
agtalk agent "结论如下..." -n reviewer -r msg_123

# 回复并标记完成（隐式 read + done）
agtalk agent "处理完成，结论如下..." -n reviewer -r msg_123 -d msg_123
```

### 5. 查看对话列表

```bash
agtalk chats
```

只列会话，不改任何消息状态。

## inbox 返回结构

```json
{
  "id": "msg_123",
  "from": {
    "id": "codex",
    "name": "codex",
    "type": "agent",
    "intro": "coding agent"
  },
  "subject": "storage.rs 重构 review",
  "content": {
    "mode": "preview",
    "body": "我完成了 session 机制重构，主要改动包括：...",
    "truncated": true,
    "size": 12480
  },
  "attachments": [
    {
      "id": "att_001",
      "role": "full_body",
      "filename": "message-msg_123.md",
      "content_type": "text/markdown",
      "size": 12480
    }
  ],
  "delivery": {
    "status": "read",
    "delivered_at": null,
    "read_at": "2026-06-18T20:30:12Z",
    "done_at": null
  },
  "actions": ["detail", "reply", "done", "attachment"],
  "action_required": false,
  "priority": "normal",
  "kind": "message"
}
```

字段说明：

- `from`：发送者信息。`intro` 来自 `agtalk join --intro`。
- `subject`：消息主题，来自 `agtalk agent -s <标题>` 或 metadata。
- `content.mode`：
  - `full`：完整正文（`size <= inbox_inline_limit_bytes`）。
  - `preview`：预览（`inbox_inline_limit_bytes < size <= attachment_threshold_bytes`）。
  - `summary`：摘要，正文已拆为附件（`size > attachment_threshold_bytes`）。
- `content.truncated`：body 是否被截断。
- `content.size`：原始 body 字节数。
- `attachments`：附件列表。`full_body` 表示超长消息的完整正文。
- `delivery.status`：`pending` / `delivered` / `read` / `done`。
- `actions`：建议操作。`approval_request` 会额外包含 `approve` / `reject`。

## 自动已读规则

不需要手动 `mark-read`。daemon 会在以下场景自动更新 delivery 状态：

| 操作 | 效果 |
|---|---|
| `agtalk inbox` | 返回的消息 → `read` |
| `agtalk inbox --peek` | 不改状态 |
| `agtalk detail <msg-id>` | 该消息 → `read` |
| `agtalk attachment <att-id>` | 对应消息 → `read` |
| `agtalk chats` | 不改状态 |
| `agtalk agent ... -r <msg-id>` | 原消息 → `read` |
| `agtalk agent ... -d <msg-id>` | 原消息 → `read` + `done` |

`read_at` 只记录第一次读取时间；`done_at` 记录完成时间。

## 附件类型

当消息正文超过 `attachment_threshold_bytes`（默认 8KB）时，daemon 会自动把完整正文存为 `full_body` 附件。你也可以在发送消息时或后续为消息附加其他类型文件（未来 API）。

| role | 使用时机 |
|---|---|
| `full_body` | 超长消息的完整正文 |
| `user_file` | 用户或 Agent 显式附加的文件 |
| `generated_report` | Agent 生成的报告 |
| `patch` | diff / patch 文件 |
| `log` | 命令输出日志 |
| `artifact` | 生成物：设计稿、文档、图片等 |

读取附件：

```bash
agtalk attachment att_001
```

## 配置

配置文件：`~/.config/agtalk/config.json`

默认值：

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

查看/修改：

```bash
agtalk config list
agtalk config get message.attachment_threshold_bytes
agtalk config set message.attachment_threshold_bytes 4096
agtalk daemon restart
```

修改阈值后需要重启 daemon 生效。

## 典型协作模式

### 模式一：任务分发与完成

```bash
# codex 给 reviewer 发任务
codex$ agtalk agent "请 review storage.rs 的 session 校验" -n reviewer -s "review request"

# reviewer 查收
reviewer$ agtalk inbox
# 看到 msg_123，状态 pending

# reviewer 完成并回复
reviewer$ agtalk agent "已通过，注意 token 过期处理" -n codex -r msg_123 -d msg_123

# codex 收件箱看到回复，状态 read/done
codex$ agtalk inbox
```

### 模式二：向人类请求审批

```bash
# codex 向 me（人类）发起审批
codex$ agtalk human "是否允许删除 target 目录？" -o 允许 -o 拒绝

# 人类在 GUI 弹窗或 CLI 回复后，codex 收到 approval_response
# 若 codex 的 Ask 已超时退出，或 daemon 曾经重启，可稍后通过 wait 重新取回结果
codex$ agtalk wait <msg-id> --timeout 60
```

### 模式三：长报告协作

```bash
# reviewer 发送长 review 报告（>8KB，自动存为 full_body 附件）
reviewer$ agtalk agent "$(cat review.md)" -n codex -s "PR review"

# codex inbox 看到摘要 + attachment 入口
codex$ agtalk inbox
# 读取全文
codex$ agtalk detail <msg-id>
```

### 模式四：YAML Runner 执行复杂任务

把复杂请求写入 YAML 文件，通过 `agtalk run` 执行，避免 shell 长正文、复杂引号、多附件和沙箱授权问题。Runner 只执行 agtalk 内部命令，不执行任意 shell；YAML 中的相对路径按 YAML 文件所在目录解析。

建议把每个 Agent 生成的复杂指令固定写入 `.agtalk/runs/<agent-name>.yaml`。省略文件参数时，`agtalk run` 会读取当前 Agent 对应的固定文件，方便对同一个命令入口做授权。

```yaml
# .agtalk/runs/codex-coder-Alex.yaml
version: 1
command: agent
name: reviewer
subject: "TASK: 重构 storage.rs"
message: |
  请 review 附件中的改动，重点关注：
  1. session 校验逻辑是否完整
  2. 错误处理是否清晰
reply_to: null
done: null
notify: true
files:
  - ./src/storage.rs
  - ./docs/changelog.md
```

```bash
AGTALK_NAME=codex-coder-Alex agtalk run
```

YAML 也支持向人类提问：

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

更多命令的 YAML schema 见 `docs/commands.md`。

## 最佳实践

1. **不要手动维护 read 状态**：用 `inbox` / `detail` / `attachment` / `reply` / `done` 让 daemon 自动处理。
2. **inbox 只做决策摘要**：如果正文太长，先看 `content.mode` 和 `actions`，需要时再 `detail`。
3. **发送长内容不需要预处理**：daemon 会自动拆分 >8KB 的正文为附件。
4. **用 `--intro` 说明身份**：其他 Agent 通过 `from.intro` 快速理解你的角色。
