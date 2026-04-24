---
name: agtalk
description:
  当用户提到以下场景时使用此 skill：
  - 多 Agent 协作、Agent 间通信
  - Zellij/Tmux 多路复用器环境下的 Agent 消息传递
  - 使用 agtalk 进行任务分发和结果收集
  - 注册 Agent、查看 inbox、发送消息
  - Agent 编排、工作流自动化
  - 需要在多个终端 pane 之间协调任务
  此 skill 帮助Agent 在 agtalk 环境中高效地进行多 Agent 协作开发。

  扩展机制：当 Agent 需要执行高风险决策二次确认或完成后经验沉淀时，加载 REVIEW.md（agtalk-review）。
---

# agtalk-assistant

agtalk 是一个运行在 Zellij/Tmux 终端多路复用器环境中的多 Agent 通信框架。它让多个 Agent（运行在不同 pane 中）可以通过消息 inbox、FIFO 通知和 pane 间直接通信来协作完成任务。

## 核心概念

### 核心设计理念

> **Agent 自主处置**：此 skill 的核心理念是**让 Agent 自由工作**。
> - 发送方只负责发消息 + notify 通知，**不控制** done
> - 接收方自主查收 inbox、自由处置任务，**自己决定**何时调用 `done`
> - 禁止发送方在消息中强制要求接收方何时完成

### Agent 命名规范
格式：`{agent_tool}_{main_role}_{可记忆英文名}`

- `agent_tool`: AI 工具名（claude / kimi / opencode / gemini 等）
- `main_role`: **用户自定义角色标识**，如 `coder`、`reviewer`、`planner`、`writer`、`analyst`、`ops`、`designer` 等，不限定固定值
- `可记忆英文名`: 从预设列表选取，确保不冲突

**预设名字列表（按字母顺序）**：
Alex / Bob / Chris / David / Emma / Frank / Grace / Henry / Iris / Jack / Kate / Liam / Mary / Nick / Olivia / Paul / Quinn / Rose / Sam / Tom / Uma / Victor / Wendy / Xin / Yale / Zoe

> **名字冲突检测**：注册前必须先执行 `agtalk list` 查看已有名字，**选择未使用的名字**。

### 消息状态
- `pending` -> `delivered` -> `read` -> `done`
- `failed` 表示投递失败

### 消息格式前缀
`[TASK]`、`[REPLY]`、`[DONE]`、`[ACK]`、`[INFO]`、`[FILE]`

### 收到消息后的处置流程

> **收到 `[TASK]` 消息后**：处置完任务后，**自行执行** `agtalk done <msg_id>` 标记完成，无需等待发送方显式说明。发送方使用 `--wait-done` 时会通过 FIFO 自动感知完成状态。

---

## 通用协作原则（角色无关）

agtalk 不预设固定角色体系，`role` 和 `capabilities` 完全由用户自定义。无论你的 Agent 是 `writer`、`analyst`、`ops`、`designer` 还是其他任何角色，以下原则通用：

### 1. 任意 Agent 之间可直接沟通
不需要所有消息都经过某个"中心节点"。发现 bug 的 Agent 可以直接通知负责修复的 Agent，审查完成的 Agent 可以直接通知部署的 Agent。

### 2. 单线程任务规则
一个 Agent 同时只应处理 **1 条 active `[TASK]`**。Sender 应在收到 `done` 后再发下一条，避免 inbox 堆积。

### 3. `[TASK]` 默认发送提醒
`send`/`broadcast`/`multicast` 默认 `notify=True`，目标 pane 会自动收到通知。不需要手动加 `--notify`。

```bash
# 正确 — 默认提醒
agtalk send claude_writer_Bob "[TASK] 撰写文档"

# 错误 — 对方可能收不到（只有 --no-notify 时才不提醒）
agtalk send claude_writer_Bob "[TASK] 撰写文档" --no-notify
```

### 4. `[INFO]` / `[REMINDER]` 应使用 `notify`
非任务类消息不应占用 inbox 的 pending 状态：

```bash
# 正确 — 仅提醒，不占用 inbox
agtalk notify claude_writer_Bob "[INFO] 会议 10 分钟后开始"

# 错误 — 对方需要手动 done 一条纯信息
agtalk send claude_writer_Bob "[INFO] 会议 10 分钟后开始"
```

### 5. 合理使用优先级
| priority | 适用场景 |
|----------|----------|
| 1-2 | 紧急/阻塞性任务 |
| 3-4 | 重要但不紧急 |
| 5 | 默认，常规任务 |
| 6-9 | 低优先级/可延后 |

### 6. 使用 `reply_to` 建立对话线程

```bash
# 先发送任务
agtalk send claude_analyst_Bob "[TASK] 分析数据集 A"
# 回复时带上 msg_id（从 inbox 获取）
agtalk send claude_planner_Alice "[REPLY] 分析完成，关键发现..." --reply-to <msg_id>
```

---

## 常用命令

### 环境初始化
```bash
agtalk init
```
检查终端环境、初始化数据库、确保 FIFO 存在。

### Agent 注册
```bash
# 注册当前 pane 为 Agent
agtalk register <agent_name> --role <role> --capabilities <capabilities> --bio <bio> --workdir <dir>

# 示例
agtalk register claude_coder_Alice --role coder --capabilities "python,frontend,refactor" --bio "主攻前端重构"
# 自动记录：工作目录（默认当前目录）、多路复用器类型（zellij/tmux）
```

### 查看 Agent 列表
```bash
agtalk list                    # JSON 输出
agtalk list --view             # 工作牌视图（带在线状态）
agtalk list --capabilities     # 显示能力列表
```

工作牌视图显示：角色、能力、简介、工作目录、终端信息（session:pane [mux]），在线 ● / 离线 ○ / 未知 ?。

### 发送消息

#### 1. 命令行发送（适合短消息）
```bash
# 基本发送（默认自动提醒对方）
agtalk send <agent_name> "<消息内容>"

# 不发送 pane 提醒
agtalk send <agent_name> "<消息内容>" --no-notify

# 等待对方完成（必须设置超时）
agtalk send <agent_name> "<消息内容>" --wait-done --timeout 120

> **必须设置超时**：建议 `--timeout 60` 或 `--timeout 120`，避免无限等待。若超时可重新发送消息。

# 带主题和优先级
agtalk send <agent_name> "<消息内容>" --subject "重构任务" --priority 3
```

#### 2. 从 stdin 发送（适合代码/长文本）
```bash
# 管道发送
 echo "长文本内容..." | agtalk send <agent_name> -

# 发送代码文件
cat main.py | agtalk send <agent_name> -

# heredoc 发送
agtalk send <agent_name> - << 'EOF'
多行内容
第二行
EOF
```

#### 3. 从 JSON 文件发送（适合需要配置 priority/subject/reply_to 时）
```bash
# JSON 文件格式
{
  "agent": "kimi_developer_Xin",
  "body": "长文本...",
  "subject": "重构任务",
  "priority": 3,
  "reply_to": "可选msg_id",
  "wait_done": false,
  "timeout": 120
}

# 发送
agtalk send --file message.json
# 命令行 agent 可覆盖 JSON 中的值
agtalk send claude_tester_Chris --file message.json
```

#### Shell 安全注意事项

消息内容中包含特殊字符时，bash 可能会意外处理：

```bash
# 错误 — $100 会被 bash 替换为空变量
agtalk send <agent> "价格是 $100"

# 正确 — 使用单引号阻止变量替换
agtalk send <agent> '价格是 $100'

# 正确 — 使用 \n 插入换行（agtalk 会自动转义）
agtalk send <agent> '第一行\n第二行\n第三行'

# 正确 — 使用 $'...' 语法（bash 原生支持转义）
agtalk send <agent> $'第一行\n第二行'
```

### 广播和多播
```bash
# 广播给所有 Agent（默认提醒）
agtalk broadcast "<消息内容>"

# 广播但不提醒
agtalk broadcast "<消息内容>" --no-notify

# 排除某些 Agent
agtalk broadcast "<消息内容>" --exclude "agent1,agent2"

# 多播给指定 Agent（默认提醒）
agtalk multicast "agent1,agent2" "<消息内容>"
```

### 提醒通知（仅触发 inbox 查收）
```bash
# 向对方 pane 发送提醒，通知格式：
# [agtalk:<msg_id>] | exec: agtalk inbox <your_name>
agtalk notify <agent_name>

# 自定义提醒内容
agtalk notify <agent_name> "请查收任务"

# 不自动发送 Enter（让对方手动触发）
agtalk notify <agent_name> --no-enter
```

> **注意**：notify 只负责通知对方查收 inbox，done 由 Agent 自己处理完任务后决定何时调用。

### 查看 Inbox
```bash
# 查看当前 Agent 的收件箱
agtalk inbox <your_name>

# 包含已读消息
agtalk inbox <your_name> --all

# 人类可读视图
agtalk inbox <your_name> --view
```

### 标记完成
```bash
# 标记消息已完成
agtalk done <msg_id>
```

### Kanban 看板
```bash
# 创建任务卡片
agtalk kanban post "优化查询性能" "需要给 users 表加索引"

# 查看看板（人类可读）
agtalk kanban list --view

# 发布公告
agtalk kanban announce "API 变更通知" "v2 接口已上线"

# 查看卡片详情
agtalk kanban show <card_id>

# 添加评论
agtalk kanban comment <card_id> "加索引后 QPS 提升 3x"

# 移动卡片状态
agtalk kanban move <card_id> in_progress

# 关闭卡片
agtalk kanban close <card_id>
```

### 进度追踪
```bash
# 更新当前 Agent 的工作状态
agtalk progress <agent_name> "正在重构 auth 模块"

# 清除状态（标记为空闲）
agtalk progress <agent_name> ""
```

### 健康检查
```bash
# 系统健康检查
agtalk health

# 指定 Agent 健康检查
agtalk health --agent <agent_name>
```

### 清理工具
```bash
# 清理僵尸 Agent
agtalk prune

# 干跑模式（不实际删除）
agtalk prune --dry-run

# 检查卡死消息
agtalk check-stuck

# 查看消息历史
agtalk memory --last 50

# 单条消息详情
agtalk memory <msg_id>

# 任务视图（按对话分组，人类可读）
agtalk memory --group
```

---

## 典型工作流

### 1. 任务分发与收集
```bash
# Agent A 分发任务给 B 和 C
agtalk send claude_coder_Bob "[TASK] 请重构 auth 模块" --priority 3
agtalk send claude_writer_Chris "[TASK] 请撰写技术文档" --priority 5

# B 和 C 完成后执行
agtalk done <msg_id>
```

### 2. 多 Agent 流水线
```bash
# 流水线：coder -> reviewer -> deployer
agtalk send claude_coder_Alice "[TASK] 实现登录功能"
# Alice 完成后
agtalk send claude_reviewer_Bob "[TASK] 审查 Alice 的代码"
# Bob 完成后
agtalk send claude_ops_Charlie "[TASK] 部署到 staging"
```

### 3. 并行任务收集
```bash
# 并行分发给多个 Agent
agtalk multicast "alice,bob,charlie" "[TASK] 各自分析一个数据集"

# 等待结果
agtalk inbox me --all
```

### 4. 团队广播
```bash
# 广播部署通知
agtalk broadcast "[INFO] 即将部署到 staging 环境" --exclude "ci_bot"

# 广播紧急停止
agtalk broadcast "[TASK] 紧急：停止当前所有操作"
```

---

## 与 AI Agent 集成

当你在 Zellij/Tmux 环境中使用 Claude Code、Kimi 或其他 AI Agent 时，agtalk 可以实现：

1. **并行工作**：在多个 pane 启动不同的 Agent，分别负责任务的不同部分
2. **专业分工**：不同 Agent 根据自定义角色各司其职（开发、写作、分析、运维等）
3. **结果汇总**：通过 inbox 收集各 Agent 的工作结果

### 在 Zellij 中设置多 Agent
```bash
#Pane 1: 注册为 coder
cd /path/to/project
agtalk register claude_coder_Alice --role coder
claude

#Pane 2: 注册为 writer
cd /path/to/project
agtalk register claude_writer_Bob --role writer
claude

#Pane 3: 主控/协调者
agtalk register claude_planner_Charlie --role planner
# 分发任务
agtalk send claude_coder_Alice "[TASK] 实现用户模块"
agtalk send claude_writer_Bob "[TASK] 撰写 API 文档"
```

---

## 故障排除

### "不在 Zellij/Tmux 环境中"
确保在 Zellij session 或 Tmux session 内运行。Zellij 会设置 `ZELLIJ_SESSION_NAME`，Tmux 会设置 `TMUX`。

### "Agent 未找到或已离线"
- 检查对方是否已注册：`agtalk list`
- 对方 pane 是否还活着
- 尝试重新注册

### 消息未送达
- 检查 FIFO 是否存在：`ls -la ~/.config/agtalk/`
- 重新初始化：`agtalk init`

### 僵尸 Agent
```bash
# 查看并清理
agtalk prune --dry-run
agtalk prune
```

---

## 数据存储

所有数据存放在 `~/.config/agtalk/` 目录下：

| 文件/目录 | 说明 |
|-----------|------|
| `~/.config/agtalk/talk.db` | SQLite 数据库（消息、Agent 注册、日志） |
| `~/.config/agtalk/notify.fifo` | FIFO 管道（pane 间通知） |

### 数据库结构
- `agents` 表：已注册的 Agent 信息（含 workdir、mux）
- `messages` 表：消息 inbox
- `message_log` 表：消息事件日志
- `offline_queue` 表：离线消息队列
- `kanban_cards` 表：看板卡片和公告
- `kanban_comments` 表：看板评论

### 手动访问数据库
```bash
# 查看数据库内容
sqlite3 ~/.config/agtalk/talk.db ".tables"
sqlite3 ~/.config/agtalk/talk.db "SELECT * FROM agents;"

# 自定义数据库路径（覆盖默认）
export AGTALK_DB_PATH=/path/to/custom.db
```

---

## 环境变量

| 变量 | 说明 |
|------|------|
| `AGTALK_AGENT_NAME` | 当前 Agent 名称 |
| `ZELLIJ_SESSION_NAME` | Zellij session 名 |
| `ZELLIJ_PANE_ID` | 当前 pane ID |
| `TMUX` | Tmux 环境标志 |
| `AGTALK_DB_PATH` | 自定义数据库路径（默认 `~/.config/agtalk/talk.db`） |

---

## agtalk 消息处理流程（System Prompt 定义）

当收到 `[agtalk:*]` 通知时，按以下流程处理：

```
1. 收到 [agtalk:<msg_id>] 通知后，执行 exec 里的命令查收 inbox
   agtalk inbox <your_name>

2. 读取消息内容，执行消息要求的任务

3. 任务完成后，执行 agtalk done <msg_id> 标记完成
```

> **禁止在任务未处理前执行 done**。done 由 Agent 自己决定何时调用，不应由发送方控制。

---

## 扩展：agtalk-review

当 Agent 遇到以下场景时，加载同目录下的 `REVIEW.md`（agtalk-review skill）：

- **高风险/模糊决策** — 需要阻塞等待其他 Agent 二次确认后再继续
- **完成后反思** — `agtalk done` 前自检沟通质量，将通用经验沉淀回 `REVIEW.md`

`agtalk-review` 是 `agtalk-assistant` 的可选扩展，Agent 基于自身判断自主决定是否发起 Review。
