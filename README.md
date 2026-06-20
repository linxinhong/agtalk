# agtalk

**本地 Agent 对话总线 + Human-in-the-loop 协作工具**

agtalk 不是普通聊天应用，而是让 CLI Agent、GUI、弹出窗口和人类之间可靠传话、追踪投递状态、保存上下文的本地通信中枢。daemon 是所有状态的唯一真相来源，CLI 与 GUI 均为薄客户端。

## 核心场景

- **Agent → Human 审批**：Agent 执行风险操作前，通过弹窗或 GUI 向人类请求确认。
- **Human → Agent 指令**：人类从 GUI 或 CLI 向本地 Agent 发送结构化任务消息。
- **Agent ↔ Agent 协作**：多个本地 Agent 按任务线程交换结构化消息。
- **GUI 工作台**：收件箱、会话历史、审批面板统一入口。

## 架构

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

## 安装

```bash
# 1. 克隆仓库
git clone https://github.com/linxinhong/agtalk.git
cd agtalk

# 2. 安装前端依赖
pnpm install

# 3. 编译 Rust 二进制
cargo build --release

# 4. 部署到 ~/.local/bin（可选）
make deploy
```

前置依赖：Rust、Node.js/pnpm。

## 快速开始

```bash
# 启动 daemon
agtalk daemon start

# Agent 加入网络
agtalk join codex --intro "coding agent"
export AGTALK_NAME=codex

# 向人类发起审批
agtalk ask @me "是否允许删除 target 目录？" --choices approve,reject

# 人类通过弹窗/GUI/CLI 回复
agtalk inbox
agtalk reply <message-id> approve

# Agent 查看收件箱
agtalk inbox
```

## 常用命令

| 命令 | 说明 |
|------|------|
| `agtalk daemon start` | 后台启动 daemon |
| `agtalk daemon status` | 查看 daemon 状态 |
| `agtalk join <name>` | Agent 加入网络 |
| `agtalk leave [--purge]` | 离开网络，可选清除本地凭证 |
| `agtalk cleanup [--dry-run]` | 清理当前 workspace 已退役 session |
| `agtalk agent <消息>` | 给 Agent 发任务 / 回复 |
| `agtalk human <消息>` | 向人类发送消息或提问 |
| `agtalk ask @me <问题>` | 发起带选项的审批 |
| `agtalk inbox` | 查看收件箱（自动标记已读） |
| `agtalk reply <msg-id> <choice>` | 回复审批/消息 |
| `agtalk chats` | 列出会话 |
| `agtalk gui` | 启动 Tauri GUI |
| `agtalk run [file.yaml]` | 通过 YAML Runner 执行命令 |

完整命令参考见 [`docs/commands.md`](docs/commands.md)。

## 开发

```bash
# 前端开发服务器
pnpm dev

# Tauri 开发模式
pnpm tauri dev -- gui

# 运行 Rust 单元测试
cargo test --bin agtalk

# 生产构建
make release
```

## 文档

- [`docs/DESIGN.md`](docs/DESIGN.md)：架构设计与核心概念
- [`docs/commands.md`](docs/commands.md)：CLI 命令完整参考
- [`docs/agtalk-manual.md`](docs/agtalk-manual.md)：Agent 使用手册

## 协议

[MIT](LICENSE)
