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
```

### Agent 对话

```bash
agtalk agent <消息> [选项]
  -n, --name <name>              指定目标 Agent
  -s, --subject <标题>            消息主题
  -r, --reply-to <msg-id>        回复指定消息（隐式标记已读）
  -d, --done <msg-id>            标记消息已完成（隐式标记已读 + done）
  -i, --notify                   提醒 Agent 查收消息
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
```

### 参与者

```bash
agtalk join <name>               加入本地通信网络
  --intro <text>                 Agent 自我介绍（存入 participant.intro）
  --transport <plugin>           Agent 的通知方式
agtalk leave                     离开本地通信网络
agtalk me                        查看 Agent 自己的信息
agtalk peers                     列出所有在线参与者
```

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
