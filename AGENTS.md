# AGENTS.md — agtalk 开发要求

本文档是 agtalk 项目的开发纪律，面向所有参与者（包括 AI agent）。**所有要求必须遵守**，违反前先讨论。架构原理见 `docs/design.md`。

---

## 1. 项目定位

agtalk 是**本地 Agent 对话总线**。daemon 是唯一真相来源，CLI / GUI / 浏览器扩展都是薄客户端。

- 不复用 `~/projects/agtalk-office` 的代码（那是早期实现，本项目参考其设计思路但全新实现）。
- 单二进制 `agtalk`，argv 分派：`daemon`（后台进程）/ `gui`（Tauri GUI）/ `__popup`（审批弹窗）/ 其它（CLI 子命令）。
- 三个对话域（agent-agent / agent-human / agent-browser）必须用**同一套机制**（mailbox + SSE），不允许加域特例。

---

## 2. 架构红线（不可违反）

1. **路由只认 UUID**。`send(to=address)`，address 是 UUID。任何按 name 路由的代码都是错的。
2. **name 不唯一，纯展示**。name 永远不进路由查询。消歧在调用方，用 lookup 返回的 intro+workspace。
3. **身份载体 = 文件系统**（`.agtalk/<name>/session.json` + `agents.json`）。**禁止**让 agent 持有/记忆高熵 token 作为认证锚（compact 会丢）。认证链：PID → agents.json → name → session.json → UUID。
4. **SSE 是唯一推送机制**。**禁止**引入长轮询/短轮询/双机制并存。
5. **消息推送前必须先持久化**（at-least-once）。event_id 单调，支持 Last-Event-ID 重放。
6. **三域统一**：human/browser 不是特例，都是不同生命周期的 mailbox。
7. **mailbox 生命周期 = 文件夹生命周期**。daemon 的 lookup 表是文件系统的镜像，不是独立真相源。消除身份用 `agtalk leave`（实时）+ 惰性清理（兜底）。

---

## 3. 目录与模块结构

### 3.1 workspace 多 crate

```
crates/
  agtalk/        ← bin，只做 argv 分派，main.rs < 100 行
  agtalk-core/   ← lib，核心逻辑
```

**禁止**在 bin crate 里写业务逻辑。bin 只 dispatch 到 core。

### 3.2 core 按领域分模块

```
agtalk-core/src/
  proto.rs        ← ClientMsg/ServerMsg enum 定义（协议内聚）
  identity/       ← mailbox、session.json、agents.json、PID 解析
  routing/        ← send、lookup
  transport/      ← SSE 端点、唤醒机制
  server/         ← HTTP/socket 入口（薄）
  storage/        ← DB 句柄、迁移（不塞业务查询）
  config.rs       ← AgConfig
```

**禁止**：
- 把所有领域塞进一个文件（god-object）。
- 在 storage 模块里写业务查询（查询分散到各领域模块）。
- 在 proto.rs 里写 handler（只放 enum 定义）。

### 3.3 文件大小硬约束

- **目标**：单文件 < 500 行。
- **红线**：绝不超 800 行。超过必须拆分。
- `handle_msg` 这类大 match：**按 ClientMsg 分组拆成多个 handler 函数/文件**，主 match 只做分发。绝不写 1500 行的 match。

---

## 4. 编码规范

### 4.1 Rust

- `rustfmt` 默认风格。`cargo clippy -- -D warnings` 必须通过。
- 模块名 `snake_case`，类型 `PascalCase`。
- **错误处理**：用 `thiserror` 定义每个领域模块自己的 Error 类型。**禁止**全项目一个 `anyhow::Error` 一把梭。daemon 边界统一转 `ServerMsg::Error`。
- **锁**：`Mutex::lock()` 不裸 `unwrap()`——用 `.unwrap_or_else(|e| e.into_inner())` 处理中毒 mutex，避免 daemon 因一次 panic 永久卡死。
- **IPC 协议**：`#[serde(tag = "type", rename_all = "snake_case")]`，newline-JSON。
- **PID 复用防护**：所有 `agents.json` 操作必须带 `start_time`，daemon 校验 pid + start_time 双因子。

### 4.2 协议演进纪律

每加一个 ClientMsg 变体，四处同改，加 checklist 注释：
1. `proto.rs` 定义
2. 对应领域 handler
3. CLI 子命令（如需要）
4. 测试

### 4.3 TypeScript（前端/扩展）

- `vue-tsc --noEmit` / `tsc --noEmit` 必须通过（strict 模式）。
- 类型优先用显式定义，避免 any。

---

## 5. 测试要求

### 5.1 测试就近原则

- 每个领域模块内 `#[cfg(test)] mod tests`，测试紧贴实现（`identity/tests.rs`、`routing/tests.rs` 等）。
- 跨模块集成测试放在 `agtalk-core/tests/` 或 `agtalk/tests/`。
- **禁止**把所有测试堆进一个 `tests.rs`（agtalk-office 的教训）。

### 5.2 测试覆盖底线

每个新功能必须有测试。重点关注：
- 身份认证链（PID → agents.json → session.json → UUID）
- 路由（send 按 UUID 投递，不按 name）
- SSE 推送（唤醒、Last-Event-ID 重放、断线不丢）
- mailbox 生命周期（创建、leave、惰性清理）
- PID 复用防护

### 5.3 隔离

每个测试用独立的内存 SQLite + 独立的临时 `.agtalk/` 目录（`tempfile::tempdir()`）。不共享状态。测试结束自动清理（**必须**用 tempfile 自动清理，不要手动拼路径遗留临时文件）。

---

## 6. 构建与验证命令

```bash
# Rust
cargo check                          # 快速编译检查
cargo build                          # debug 构建
cargo build --release --features custom-protocol   # release（直接 cargo build 必须
                                                     # 开 custom-protocol，否则 GUI 白屏）
cargo test                           # 全部测试（bin + lib）
cargo clippy -- -D warnings          # lint，零警告
cargo fmt --check                    # 格式检查

# 前端
pnpm install
pnpm build                           # 内含 vue-tsc --noEmit
pnpm dev                             # Vite dev server

# 扩展
cd extension && pnpm install && pnpm build

# Tauri dev
pnpm tauri dev -- gui

# Daemon
./target/debug/agtalk daemon start
./target/debug/agtalk daemon status
```

**提交前必过**：`cargo check` + `cargo test` + `cargo clippy -- -D warnings` + `pnpm build`。

---

## 7. CI 要求

**从第一天就建 CI**（`.github/workflows/`）。最小内容：
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- `pnpm build`（前端类型检查）

不建 CI、依赖人工跑检查是 agtalk-office 的教训，本项目不重犯。

---

## 8. Git 与提交规范

- **提交信息用中文**，简洁描述性。
- 一个 commit 一个逻辑变更。
- 分支：feature 分支开发，main 保持可发布。
- `.gitignore` 必须覆盖：`/target/`、`/node_modules/`、`/dist/`、`.agtalk/`、`*.db*`、`*.token`、`__pycache__/`、`.DS_Store`。
- **禁止提交**：1MB 以上的临时转储文件（如对话 HTML 导出）、构建产物、运行时状态。

---

## 9. 安全要求（本地单用户威胁模型）

- 文件权限：`.agtalk/<name>/session.json` 权限 0600（参考 agtalk-office session.rs 的 `set_permissions_0600`）。DB 文件打开后设 0600。
- daemon 监听仅 `127.0.0.1`，不对外。
- 认证锚点是文件系统（能读 session.json 的同 uid 进程即被信任），daemon 叠加 PID+start_time 校验。
- notify（终端通知）只注入**短 ID 或可读名**，**禁止注入消息正文**到终端（防注入）。
- 外部 notify 命令插件路径必须绝对，参数数组执行（不经 shell）。

---

## 10. 文档要求

- `docs/design.md` 是架构单一真相源。架构变更先改 design.md。
- 每个子系统（identity / routing / transport）成熟后补独立说明文档。
- SSE 参考实现见 `docs/sse-demo/`（含 README，说明它验证什么、agtalk 需补什么）。
- CLI 命令完整参考：随实现补 `docs/commands.md`。

---

## 11. 工作流

1. 改架构前先读 `docs/design.md`。
2. 写代码前先确认符合本 AGENTS.md 的红线。
3. 新功能先写测试（领域模块 tests.rs）。
4. 提交前跑齐验证命令（第 6 节）。
5. 文件超 500 行立即考虑拆分。
6. 遇到"特例"（某个域需要特殊处理）立即停下来——多半是架构没想清楚，先回 design.md 重新设计，而不是加 if 分支。
