# agtalk 设计文档

> **本地 Agent 对话总线 + Human-in-the-loop 协作工具**
> 不是单纯聊天软件，而是让 Agent、CLI、GUI、人类审批窗口之间可靠传话、追踪状态、保存上下文的本地通信中枢。

## 架构模型

```
CLI / Agent / GUI
      │
      ▼
agtalk daemon
      │
      ├─ SQLite：身份、会话、消息、状态、审计
      ├─ Router：消息路由、投递、重试、等待响应
      ├─ Transport：Terminal / Popup / GUI / Agent Adapter
      └─ IPC：Unix socket newline-JSON
```

daemon 是所有状态的唯一真相来源。CLI 和 GUI 都是薄客户端。

## 核心对象

### Participant（参与者）

同时支持人和 Agent，不只是 username。

| 字段 | 说明 |
|------|------|
| id | 唯一标识 |
| name | 如 @me, @security-agent, @codex, @reviewer |
| kind | human / agent / tool / system |
| transport | terminal / popup / gui / cli / http / process |
| endpoint | 传输端点 |
| capabilities | 能力标签 |
| status | online / offline / busy |

### Conversation（会话）

不止一对一聊天，预留多种会话类型。

| kind | 用途 |
|------|------|
| direct | agent 和人直接问答 |
| group | 多个 agent + human 协同 |
| task | 某个任务执行过程 |
| approval | 等待人类确认 |
| incident | 安全告警研判闭环 |

### Message（消息）

支持结构化事件，比普通聊天消息更强。

| 字段 | 说明 |
|------|------|
| correlation_id | 关联请求和响应，跨消息追踪 |
| message_type | text / markdown / event / command / tool_call / tool_result / approval_request / approval_response / artifact / error |
| content | 文本内容 |
| content_json | 结构化数据 |
| status | pending / delivered / read / done / failed / expired |

### Delivery（投递状态）

一条消息可投递给多个对象，每个对象有独立状态。

```
pending → delivered → read → done
                          → failed
```

## Agent↔Human 场景（核心价值）

```
Agent 执行任务
  ↓
遇到风险/不确定/需要确认
  ↓
agtalk 弹出窗口/CLI inbox/GUI inbox
  ↓
人类确认
  ↓
Agent 继续执行
```

```bash
agtalk ask @me "是否允许删除 target 目录？" --choices approve,reject
agtalk inbox
agtalk reply <message-id> approve
agtalk wait <message-id>
```

## Agent↔Agent 协作

不搞自由聊天，先做任务协作：

```
Agent A 发起 task → Agent B 接收 → Agent B 返回 result → Agent A 继续
```

事件流：`task_created → task_assigned → task_progress → task_result → task_done`

## MVP 路线

| 版本 | 目标 |
|------|------|
| v0.1 | 可靠本地消息总线：注册参与者、发消息、查 inbox |
| v0.2 | Human-in-the-loop：ask、wait、choices、timeout |
| v0.3 | GUI Inbox：Tauri 界面，会话列表、待处理、一键确认 |
| v0.4 | Agent Adapter：接入一个真实 CLI Agent |
| v0.5 | Task Conversation：从聊天升级为任务协作 |

## 核心能力

不要问"怎么做聊天工具"，而问：

> 当一个 Agent 执行任务时，如何可靠地找到人、找到另一个 Agent、等待回应、记录过程、恢复上下文？

```
身份 · 会话 · 消息 · 投递 · 等待 · 确认 · 审计 · 恢复
```
