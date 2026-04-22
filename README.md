# agtalk — Agent Talk

[![Python](https://img.shields.io/badge/python-3.10%2B-blue)](https://www.python.org/)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**多 Agent 通信框架** — 基于终端多路复用器（Zellij / Tmux）的 Agent 间消息传递系统。

在多个终端 pane 中运行的 AI Agent（如 Claude、Kimi 等）可以通过 `agtalk` 相互发送消息、广播任务、查看收件箱，实现真正的多 Agent 协作。

---

## ✨ 功能特性

- **Agent 注册与发现** — 将终端 pane 注册为 Agent，自动维护在线状态
- **消息通信** — 支持单播、广播、多播，带优先级和状态追踪
- **Inbox 收件箱** — 类似邮件的收件箱模型，支持标记完成 (`done`)
- **实时提醒** — 通过多路复用器 `write-chars` 向目标 pane 发送通知
- **离线队列** — Agent 离线时消息进入队列，上线后自动提醒
- **健康检查** — DB、FIFO、进程存活、僵尸 Agent 清理
- **Tmux / Zellij 双支持** — 目前 Zellij 功能完整，Tmux 为预留框架

---

## 📦 安装

```bash
# 从源码安装
pip install -e .

# 或直接运行脚本
./scripts/agtalk
```

**依赖**
- Python ≥ 3.10
- [Click](https://click.palletsprojects.com/) ≥ 8.0
- [Zellij](https://zellij.dev/)（当前主要支持）

---

## 🚀 快速开始

### 1. 初始化环境

```bash
agtalk init
```

检查终端环境、初始化 SQLite 数据库、创建 FIFO 通知管道。

### 2. 注册 Agent

每个 pane 中的 Agent 需要先注册：

```bash
agtalk register claude_coder_Alex --role coder --capabilities python,review
```

> **命名规范**: `{tool}_{role}_{Name}`  
> 示例：`claude_coder_Alex`、`kimi_reviewer_Bob`

### 3. 发送消息

```bash
# 发送消息（仅写入 inbox，不弹窗提醒）
agtalk send Alex "请帮我 review 这段代码"

# 发送后自动提醒对方查收
agtalk send Alex "紧急任务" --notify

# 发送任务并等待对方完成
agtalk send Alex "重构 utils.py" --wait-done --timeout 60

# 广播给所有 Agent
agtalk broadcast "会议 10 分钟后开始" --notify

# 多播给指定 Agents
agtalk multicast "Alice,Bob" "请各自检查 CI"
```

### 4. 查看收件箱

```bash
# 查看未读消息
agtalk inbox Alex

# 查看所有消息（含已读）
agtalk inbox Alex --all
```

### 5. 标记完成

处理完消息后，标记为完成：

```bash
agtalk done <msg_id>
```

支持短 ID 前缀匹配（如 `agtalk done 7a8b9335`）。

---

## 📋 命令速查

### 注册管理
| 命令 | 说明 |
|------|------|
| `agtalk register <name>` | 注册当前 pane 为 Agent |
| `agtalk unregister <name>` | 注销 Agent |
| `agtalk list` | 列出所有注册 Agent |
| `agtalk list --capabilities` | 显示 Agent 能力列表 |
| `agtalk whoami` | 显示当前 Agent 信息 |

### 消息通信
| 命令 | 说明 |
|------|------|
| `agtalk send <agent> <body>` | 发送消息到 inbox |
| `agtalk send <agent> <body> --notify` | 发送并提醒 |
| `agtalk send <agent> <body> --wait-done` | 发送并等待完成 |
| `agtalk notify <agent> [text]` | 仅发送 pane 提醒（不写 inbox） |
| `agtalk broadcast <body>` | 广播给所有 Agent |
| `agtalk multicast <agents> <body>` | 多播（逗号分隔） |
| `agtalk inbox <name>` | 查看收件箱 |
| `agtalk done <msg_id>` | 标记消息完成 |
| `agtalk key-enter <agent>` | 向 pane 发送 Enter 键 |

### 系统工具
| 命令 | 说明 |
|------|------|
| `agtalk init` | 初始化环境 |
| `agtalk prune` | 清理僵尸 Agent |
| `agtalk check-stuck` | 标记超时未处理的消息为 failed |
| `agtalk health [agent]` | 健康检查 |
| `agtalk memory [--agent <name>]` | 查看消息历史日志 |

---

## 🏗️ 架构

```
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│   CLI       │────▶│  Messenger   │────▶│   SQLite DB     │
│  (Click)    │     │  (消息逻辑)   │     │  ~/.config/...  │
└─────────────┘     └──────────────┘     └─────────────────┘
       │                    │
       ▼                    ▼
┌─────────────┐     ┌──────────────┐
│  Registry   │     │  Delivery    │
│ (Agent管理)  │     │ (FIFO/pane)  │
└─────────────┘     └──────────────┘
       │                    │
       ▼                    ▼
┌─────────────────────────────────────────┐
│        AbstractMultiplexer              │
│   ┌─────────────┐   ┌─────────────┐    │
│   │   Zellij    │   │    Tmux     │    │
│   │  (完整实现)  │   │  (预留框架)  │    │
│   └─────────────┘   └─────────────┘    │
└─────────────────────────────────────────┘
```

### 核心模块

| 模块 | 职责 |
|------|------|
| `agtalk/cli.py` | CLI 命令定义与入口 |
| `agtalk/messenger.py` | 消息发送、收件箱、状态流转 |
| `agtalk/registry.py` | Agent 注册、查询、验活、清理 |
| `agtalk/delivery.py` | 消息投递到 pane、FIFO 通知、离线队列 |
| `agtalk/db.py` | SQLite 数据库、Schema Migration |
| `agtalk/factory.py` | 多路复用器自动检测与工厂 |
| `agtalk/term/zellij.py` | Zellij 多路复用器操作实现 |

### 消息状态流转

```
pending → delivered → read → done
                    └──────→ failed
```

---

## 🔧 配置

| 环境变量 | 说明 | 默认值 |
|----------|------|--------|
| `AGTALK_AGENT_NAME` | 当前 Agent 名称 | 从 DB 推断 |
| `AGTALK_DB_PATH` | 数据库路径 | `~/.config/agtalk/talk.db` |

---

## 🗺️ 路线图

- [ ] Tmux 多路复用器完整实现
- [ ] 消息加密传输
- [ ] WebSocket 桥接（跨机器通信）
- [ ] 消息模板与快捷指令
- [ ] Agent 能力自动发现

---

## 📄 License

[MIT](LICENSE)
