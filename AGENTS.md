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

### 3.1 单一 crate

沿用 agtalk-office 的结构，Rust 代码集中在 `src-tauri/` 一个 crate 里：

```
src-tauri/
  Cargo.toml     ← 定义 bin `agtalk` + lib `agtalk_app`
  src/
    main.rs      ← bin，只做 argv 分派，main.rs < 100 行
    lib.rs       ← lib 入口，暴露 run_gui / run_popup / run_cli 等入口
```

业务逻辑放在 `src-tauri/src/` 下的领域模块（见 3.2）。

**禁止**在 `main.rs` 写业务逻辑。`main.rs` 只负责 argv 分派并调用 `lib.rs` 暴露的入口。

### 3.2 按领域分模块

```
src-tauri/src/
  proto.rs        ← ClientMsg/ServerMsg enum 定义（协议内聚）
  identity/       ← mailbox、session.json、agents.json、PID 解析
  routing/        ← send、lookup
  transport/      ← SSE 端点、唤醒机制
  server/         ← HTTP/socket 入口（薄）
  storage/        ← DB 句柄、迁移（不塞业务查询）
  config.rs       ← AgConfig
  commands.rs     ← Tauri 命令桥（薄，仅转发到各领域模块）
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

## 5.5 浏览器扩展开发要求（WXT + Vue 3）

浏览器扩展是 agtalk 的"agent ↔ browser"对话域实现，把网页 AI 桥接到总线。技术栈：WXT 0.18 + Vue 3 + Pinia + TailwindCSS。开发必须遵守：

### 结构与复用

- **app（全屏页）和 popup（工具条弹窗）共享一套 store / API / messaging / platform 逻辑**。**禁止**写两套镜像 store（agtalk-office 的 `app/store.ts` 490 行 + `popup/store.ts` 410 行是反面教材——逻辑重复、行为漂移）。
- 共享代码放 `src/shared/`（api/、messaging/、platform/、storage/、lib/、components/）。app 和 popup 只做 UI 壳，差异仅在布局。
- 一个 Pinia store，两个 UI 消费它。

### 消息类型纪律

- 所有 chrome 消息类型常量集中在 `src/shared/messaging/message-types.ts`。
- **禁止保留"未实现的保留常量"**（agtalk-office 有 `CHAT_TURN/AGTALK_SEND` 等标注"Phase 2 不迁移"的死常量，是噪音）。一个常量要么有 handler 实现，要么删除。

### 平台选择器（selectors）——最脆弱的区域

- content script 注入 ChatGPT/Claude/Sider/ChatGLM 等 AI 站点，依赖各站点的 DOM 选择器。**站点改版即失效**，这是固有脆性，必须用工程手段缓解：
  - 每个平台的选择器**多候选**（一组选择器按序尝试，不是一个硬编码）。
  - 选择器配置化（存储在 chrome.storage，可不改代码热更）。
  - 注入失败必须可观测（记录到 `attachment-failures` 之类的存储，UI 可见）。
- **禁止**在 content script 里用 Tailwind class（注入第三方页面会被污染）。content script 注入的 UI（发送按钮、Toast）用**独立手写 CSS**，class 前缀 `agtalk-`（参考 agtalk-office `send-buttons.css`）。

### 身份模型：1 浏览器 = 1 mailbox + 标签绑定表（详见 design §3.5）

- 整个浏览器插件 = **1 个持久 mailbox**（name 如 `"browser"`，address 为 UUID）。agtalk 侧零特殊化，浏览器就是一个普通 agent。
- 多 AI 标签同开由**插件内部绑定表**解决，不污染 agtalk 协议。
- **绑定表**（存 `chrome.storage.local`）：`AI 标签页(tabId) ↔ 绑定的 agent address(UUID)`，严格 **1:1**（一个标签绑一个 agent，一个 agent 同时只绑一个标签——避免回复路由歧义）。
- 绑定由 **popup 手动配**：用户在 popup 里选"哪个 agent 绑定哪个 AI 标签"。绑定的 value 是 agent 的 address(UUID)，与 agent 类型无关（kimi code / claude code / codex / zcode 等任意 agtalk agent 都行）。

### 路由（复用 UUID，无新概念）

- **入站**（agtalk → 浏览器）：daemon 推送消息给插件 SSE（带 `from_address` = 发送 agent 的 UUID）→ 插件查绑定表：`from_address → tabId` → content script 注入该 AI 标签。
- **出站**（浏览器 → agtalk）：AI 标签回复被 content script 捕获 → 插件查绑定表：`tabId → 绑定的 agent address` → `send(该 address, 回复)`。
- **路由键 = `from_address ↔ tabId`**。禁止引入"AI 类型/标签名"作为 agtalk 协议层路由键——那是插件内部的事。
- **未绑定 agent 发消息来**：插件忽略注入，并通过 agtalk 回复该 agent 一条错误提示"你尚未绑定到任何 AI 标签，请在插件 popup 配置"。不要静默丢弃。

### 与 daemon 的通信

- background service worker 是扩展与 daemon 的唯一桥梁，通过 HTTP（`POST 127.0.0.1:19527/api`）+ SSE（`GET /events`）通信。
- 扩展首次连接 daemon 时创建持久 mailbox（如 `agtalk join browser --intro "浏览器桥接"`），address 存 chrome.storage 跨会话复用。
- 订阅用 SSE（**禁止短轮询**——agtalk-office 用 5s 短轮询是反面教材，v2 必须用 SSE 长连接）。
- 扩展自身身份认证走 agtalk 标准机制（PID + session.json）。session 信息存 `chrome.storage.local`，日志必须脱敏。

### auto-mode（自动注入/转发）的同意边界

- auto-submit（自动点击网页 AI 发送按钮）、auto-forward（自动捕获 AI 回复转发回 daemon）是**敏感操作**——可能发送用户未授权的内容、抓取敏感回复。
- 默认关闭。开启时必须有**可见的运行时指示**（如小飞机变红、状态栏提示）。
- 状态变化（注入成功/失败、转发成功/失败）必须可观测、可回滚。

### 构建与类型

- `cd extension && pnpm build`（wxt build）。Firefox 构建用 `pnpm build:firefox`。
- `tsc --noEmit` 必须通过（strict）。**必须**有独立 typecheck 脚本（agtalk-office 无独立脚本，typecheck 反馈滞后）。
- manifest version 与 package.json version **保持一致**（agtalk-office 0.1.0 vs 0.2.1 是 bug）。

---

## 5.6 Tauri 2 开发要求（GUI 外壳）

Tauri 2 是 agtalk 的桌面外壳。**GUI 是薄客户端**，所有逻辑走 daemon（经 Tauri 命令 → daemon IPC）。开发必须遵守：

### 薄外壳原则

- `src-tauri/src/main.rs` 只做 argv 分派入口；`src-tauri/src/lib.rs` 暴露 `run_gui` / `run_popup` / `run_cli` 等入口。
- **禁止**在 `main.rs` 和 `commands.rs` 里写业务逻辑。业务逻辑放在 `src-tauri/src/` 下的各领域模块（identity / routing / transport / server / storage），`commands.rs` 只做薄桥。
- Tauri 命令（`#[tauri::command]`）只做"接收前端参数 → 调对应领域函数 → 返回结果"。命令本身不含业务判断。

### 命令桥的活性

- 每个 `#[tauri::command]` 必须真正被前端调用，且必须有测试。
- **禁止**整文件标 `#[allow(dead_code)]`（agtalk-office 的 `commands.rs` 整文件 dead_code 是反面教材——分不清是预留还是死代码）。不确定要不要的命令，先不写。

### capabilities 最小权限

- `src-tauri/capabilities/default.json` 只开真正需要的权限。
- agtalk-office 只开 `core:default` + `core:window:allow-close`（审批弹窗提交后自动关窗），这是合理的最小集，沿用。
- 每加一个 capability 要说明理由。

### custom-protocol 特性（GUI 白屏陷阱）

- 直接 `cargo build`（不经 tauri CLI）**必须**开 `--features custom-protocol`，否则二进制连 devUrl(localhost) 而非内嵌 dist，**GUI 白屏**。
- `make release` / `make deploy` 必须带此特性。
- `pnpm tauri dev` 不开此特性（走 dev server 热重载）。
- 这是 agtalk-office 踩过的坑，注释必须保留在 Cargo.toml。

### 前端（Vue 3）

- 前端代码在 `src/`（根目录），不在 `src-tauri/`。
- 用 Tauri `invoke` 调命令，包装在 `src/lib/ipc.ts`。
- **禁止**声明不用的依赖（agtalk-office 声明了 vue-i18n 但 `src/i18n/` 空置、无任何 useI18n 调用，是噪音）。要用就接入，不用就别装。
- 主题用 CSS 自定义属性（`--bg/--text/--accent` 等），支持 `prefers-color-scheme: dark`。

### 审批弹窗（__popup）

- 审批弹窗是独立 Tauri 窗口进程（daemon 的 PopupTransport spawn `agtalk __popup <msg-id>`）。
- 弹窗窗口 420×320 不可调（参考 agtalk-office）。
- 弹窗提交 choice 后自动关窗（用 `core:window:allow-close`）。
- daemon 通过 ChildMonitor 监控弹窗子进程，关闭且无回复 = dismissed。

---

## 6. 构建与验证命令

```bash
# Rust（-p agtalk 明确指定 src-tauri crate）
cargo check -p agtalk                          # 快速编译检查
cargo build -p agtalk                          # debug 构建
cargo build -p agtalk --release --features custom-protocol   # release（直接 cargo build 必须
                                                                # 开 custom-protocol，否则 GUI 白屏）
cargo test -p agtalk                           # 全部测试（bin + lib）
cargo clippy -p agtalk -- -D warnings          # lint，零警告
cargo fmt --check                              # 格式检查

# 前端
pnpm install
pnpm build                           # 内含 vue-tsc --noEmit
pnpm dev                             # Vite dev server

# 扩展
cd extension && pnpm install
cd extension && pnpm build               # wxt build
cd extension && pnpm run typecheck       # tsc --noEmit（须有此脚本）

# Tauri dev
pnpm tauri dev -- gui

# Daemon
./target/debug/agtalk daemon start
./target/debug/agtalk daemon status
```

**提交前必过**：`cargo check` + `cargo test` + `cargo clippy -- -D warnings` + `pnpm build`（前端）+ `cd extension && pnpm run typecheck`（扩展类型检查）。

---

## 7. CI 要求

**从第一天就建 CI**（`.github/workflows/`）。最小内容：
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- `pnpm build`（前端类型检查）
- `cd extension && pnpm run typecheck`（扩展类型检查）

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
