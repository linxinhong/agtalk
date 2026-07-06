# agtalk

本地 Agent 对话总线。

agtalk 让 CLI agent、网页 AI、GUI、人类之间可靠传话、追踪投递状态、保存上下文。daemon 是所有状态的唯一真相来源，CLI / GUI / 浏览器扩展都是薄客户端。

## 核心场景

- **agent ↔ agent**：多个本地 agent 交换结构化任务消息。
- **agent ↔ human**：agent 执行风险操作前向人类请求确认（Human-in-the-loop）。
- **agent ↔ browser**：浏览器扩展把网页 AI（ChatGPT/Claude 等）桥接到 agtalk 总线。

## 推荐工作流：人类只需 daemon start

agtalk 的 Agent-First 设计让人类主要只负责启动 daemon，其余由 agent 自己完成：

```bash
# 1. 人类启动 daemon
agtalk daemon start

# 2. agent 自恢复身份（幂等，可重复执行）
agtalk id join <name> --intro "<what you do>" --workspace "<project>"

# 3. agent 查 id show 确认身份
agtalk id show

# 4. agent 发信、收信、等回复
agtalk id lookup [name]
agtalk msg send <address-uuid> "<message>"
agtalk msg read                      # 取未读消息
agtalk msg wait <msg-id> --timeout 30  # 短期等特定回复

# 5. agent 每轮任务结束前必查收件箱
```

`id join` 是幂等的：session 已存在时复用原 address，只更新当前进程锚点。context compaction 后，agent 只需重新 `id join` 或 `id show` 即可恢复身份——身份在文件系统，不在 agent 脑子里。

## 快速开始

```bash
# 编译
cargo build -p agtalk

# 启动 daemon
./target/debug/agtalk daemon start

# 创建身份（幂等）
./target/debug/agtalk id join nora --intro "前端 review" --workspace "projA"

# 查看自己
./target/debug/agtalk id show

# 查找其他 agent
./target/debug/agtalk id lookup

# 发消息（按 UUID）
./target/debug/agtalk msg send <target-address> "hello"

# 查看未读消息
./target/debug/agtalk msg read
```

## 项目结构

```
agtalk/
├── src/                    # Tauri GUI 前端（Vue 3）
├── src-tauri/              # Rust：bin + lib + daemon 核心逻辑
│   ├── src/
│   │   ├── main.rs         # argv 分派入口
│   │   ├── lib.rs          # lib 入口
│   │   ├── cli/            # CLI 子命令与客户端
│   │   ├── cli/            # CLI 子命令与客户端
│   │   ├── identity/       # session、agents.json、mailbox、认证
│   │   ├── routing/        # send、lookup、inbox、reply
│   │   ├── mem/            # 计划、上下文、长期记忆
│   │   ├── run/            # YAML 编排入口
│   │   ├── transport/      # SSE、订阅者唤醒
│   │   ├── server/         # HTTP API、daemon 生命周期
│   │   ├── storage/        # SQLite 句柄与迁移
│   │   ├── notify/         # zellij/tmux 打扰通道
│   │   └── proto.rs        # ClientMsg / ServerMsg 协议
├── extension/              # 浏览器扩展（WXT + Vue 3）
├── docs/                   # 设计文档与命令参考
└── skills/agtalk-bridge/   # 给 AI agent 用的 skill
```

## 构建与验证

```bash
# Rust
cargo check -p agtalk
cargo test -p agtalk
cargo clippy -p agtalk -- -D warnings
cargo fmt --check

# 前端
pnpm install
pnpm build

# 浏览器扩展
cd extension && pnpm install && pnpm run typecheck && pnpm run build

# 端到端（浏览器扩展身份生命周期）
./scripts/e2e-browser.sh
```

## 关键设计

- **路由只认 UUID**：`send(to=<address>)`，name 永远不进入路由。
- **name 不唯一，纯展示**：多个 agent 可以同名，消歧在调用方。
- **身份载体 = 文件系统**：`.agtalk/<name>/session.json` + `agents.json`，compact 压不到。
- **SSE 是唯一推送机制**：无长轮询/短轮询。
- **消息先持久化再推送**：event_id 单调，支持 Last-Event-ID 断线重放。
- **三域统一**：human/browser 不是特例，都是不同生命周期的 mailbox。

详见 `docs/design.md` 与 `AGENTS.md`。
