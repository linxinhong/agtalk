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
   - **浏览器扩展域例外**：扩展无法访问本地 `.agtalk/` 文件系统，因此由 daemon 通过 `POST /api/v1/browser/join` 颁发高熵 token，扩展仅存于 `chrome.storage.local`；daemon 在 `browser_sessions` 表校验该 token。该例外**仅限浏览器域**，不得扩展到 CLI/GUI/agent-agent 域。
4. **SSE 是唯一推送机制**。**禁止**引入长轮询/短轮询/双机制并存。
5. **消息推送前必须先持久化**（at-least-once）。event_id 单调，支持 Last-Event-ID 重放。
6. **三域统一**：human/browser 不是特例，都是不同生命周期的 mailbox。
7. **mailbox 生命周期 = 文件夹生命周期**。daemon 的 lookup 表是文件系统的镜像，不是独立真相源。消除身份用 `agtalk leave`（实时）+ 惰性清理（兜底）。
8. **agent-first 身份选择**：`--as <name>` 与 `AGTALK_NAME=<name>` 只用于选择本地 `session.json`，不参与消息路由。`agtalk id join <name>` 是幂等的：session 存在则复用原 address，仅更新 PID/start_time 锚点；不存在才创建新 mailbox。

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
  proto.rs        ← ServerMsg enum 与共享 DTO 定义（协议内聚）
  cli/            ← CLI 子命令、HTTP 客户端、Context、输出格式化、YAML runner
  identity/       ← mailbox、session.json、agents.json、PID 解析
  routing/        ← send、lookup、inbox、wait
  transport/      ← SSE 端点、唤醒机制
  server/         ← HTTP 入口（薄）与 handler
  storage/        ← DB 句柄、迁移（不塞业务查询）
  config.rs       ← AgConfig
  paths.rs        ← 配置目录、状态路径、权限工具
  commands.rs     ← Tauri 命令桥（薄，仅转发到各领域模块）
  mem/            ← 记忆/协作状态（plan、entries、pack）
  notify/         ← 打扰层（zellij / tmux / auto 检测）
  tool/           ← 工具/诊断（doctor、version、path）
```

**禁止**：
- 把所有领域塞进一个文件（god-object）。
- 在 storage 模块里写业务查询（查询分散到各领域模块）。
- 在 proto.rs 里写 handler（只放 enum 定义）。

### 3.3 文件大小硬约束

- **目标**：单文件 < 500 行。
- **红线**：绝不超 800 行。超过必须拆分。
- HTTP handler 中大 match：**按 endpoint / action 分组拆成多个 handler 函数/文件**，主路由只做分发。绝不写 1500 行的 match。

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

每加一个 ServerMsg 变体或共享 DTO，四处同改，加 checklist 注释：
1. `proto.rs` 定义
2. 对应领域 handler
3. CLI 子命令与输出格式化（如需要）
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

## 5.4 浏览器扩展开发要求（WXT + Vue 3）

浏览器扩展是 agtalk 的"agent ↔ browser"对话域实现，把网页 AI 桥接到总线。技术栈：WXT 0.19 + Vue 3 + TypeScript。开发必须遵守：

### 结构与复用

- **background / popup / content script 共享 `src/api.ts` / `src/types.ts` 下的 api 与类型**。禁止把同一逻辑复制到多个 entrypoint。
- 共享代码集中放在：
  - `src/api.ts`：HTTP 封装（`join` / `leave` / `lookup` / `send` / SSE 订阅）。
  - `src/types.ts`：扩展内部类型。
  - `src/shared/messaging/message-types.ts`：所有 chrome runtime 消息常量（content script 接入后创建）。
  - `src/shared/platform/`：content script 平台选择器与注入逻辑（预留，当前为下一阶段做准备）。
- popup 只做 UI 壳，业务状态走 `chrome.storage.local` + background 消息。

### 消息类型纪律

- 所有 chrome 消息类型常量集中在 `src/shared/messaging/message-types.ts`。
- **禁止保留"未实现的保留常量"**。一个常量要么有 handler 实现，要么删除。

### 身份模型：1 浏览器 = 1 mailbox（当前实现）

- 整个浏览器插件 = **1 个持久 mailbox**（name 默认 `"browser-<short>"`，address 为 UUID）。agtalk 侧零特殊化，浏览器就是一个普通 agent。
- 扩展首次连接 daemon 时调用 `POST /api/v1/browser/join` 创建 mailbox，daemon 返回 `{address, name, token}`；扩展将 `address` + `token` 写入 `chrome.storage.local` 跨会话复用。
- 浏览器域认证使用 daemon 颁发的 token（见 §2 架构红线第 3 条例外），通过 header `X-AgTalk-Browser-Token` + `X-AgTalk-Address` 发送请求；`GET /api/v1/events` 同样使用该 token。

### 与 daemon 的通信

- background service worker 是扩展与 daemon 的唯一桥梁：
  - `POST /api/v1/browser/join` 创建身份。
  - `GET /api/v1/events` 订阅 SSE 推送（**禁止短轮询**）。
  - `POST /api/v1/msg/send` 发消息，`GET /api/v1/id/lookup` 做地址消歧，`POST /api/v1/id/leave` 注销。
- SSE 订阅使用 `fetch` + `ReadableStream` 手动解析，因为 `EventSource` 无法自定义认证 header。
- session 信息（address / token）存 `chrome.storage.local`，日志必须脱敏。

### 平台选择器（selectors）与标签绑定表（预留架构，详见 design §3.5）

- content script 注入 ChatGPT/Claude 等 AI 站点，依赖各站点的 DOM 选择器。**站点改版即失效**，这是固有脆性，必须用工程手段缓解：
  - 每个平台的选择器**多候选**（一组选择器按序尝试，不是一个硬编码）。
  - 选择器配置化（存储在 chrome.storage，可不改代码热更）。
  - 注入失败必须可观测（记录到 `attachment-failures` 存储，UI 可见）。
- **禁止**在 content script 里用 Tailwind class（注入第三方页面会被污染）。content script 注入的 UI 用**独立手写 CSS**，class 前缀 `agtalk-`。
- 标签绑定表设计：`AI 标签页 (tabId) ↔ 绑定的 agent address(UUID)`，严格 **1:1**。该功能当前处于接口预留阶段，实现时复用已有 UUID 路由，禁止引入"AI 类型/标签名"作为 agtalk 协议层路由键。
- **未绑定 agent 发消息来**：插件忽略注入，并通过 agtalk 回复该 agent 一条错误提示"你尚未绑定到任何 AI 标签，请在插件 popup 配置"。不要静默丢弃。

### auto-mode（自动注入/转发）的同意边界

- auto-submit（自动点击网页 AI 发送按钮）、auto-forward（自动捕获 AI 回复转发回 daemon）是**敏感操作**——可能发送用户未授权的内容、抓取敏感回复。
- 默认关闭。开启时必须有**可见的运行时指示**。
- 状态变化（注入成功/失败、转发成功/失败）必须可观测、可回滚。

### 构建与类型

- `cd extension && pnpm build`（wxt build）。
- `cd extension && pnpm run typecheck`（`vue-tsc --noEmit`）必须通过（strict）。
- manifest version 与 package.json version **保持一致**。

---

## 5.5 Tauri 2 开发要求（GUI 外壳）

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
- **禁止**声明不用的依赖。要用就接入，不用就别装；已声明的依赖必须在代码中有实际调用。
- 主题用 CSS 自定义属性（`--bg/--text/--accent` 等），支持 `prefers-color-scheme: dark`。

### 审批弹窗（__popup）

- 审批弹窗是独立 Tauri 窗口进程（daemon 的 PopupTransport spawn `agtalk __popup <msg-id>`）。
- 弹窗窗口 420×320 不可调（参考 agtalk-office）。
- 弹窗提交 choice 后自动关窗（用 `core:window:allow-close`）。
- daemon 通过 ChildMonitor 监控弹窗子进程，关闭且无回复 = dismissed。

---

## 5.6 打扰层（notify）开发要求

notify 是 agtalk 解决"agent 会偷懒"的机制：daemon 有新消息时**主动**把"有消息"信号推到 agent 的执行环境。详见 design §5。开发必须遵守：

### 红线

1. **notify 只推"有消息"信号，绝不推正文**（防 shell 注入）。注入文本只含信号 + 取信命令模板，使用当前二进制路径，例如 `[agtalk] 新消息来自 nora，运行 /Users/.../agtalk msg read 查看`。
2. **notify 是 pull 的互补，不是替代**。notify 失败时退化为纯 pull，消息仍在 DB 不丢。禁止把 notify 设计成"唯一投递路径"。
3. **agent 可关闭 notify**（`join --notify none`）。禁止强制打扰。

### 多通道实现

agent 跑在不同环境，notify 必须多通道，按 agent `join` 时声明的 `--notify <channel>` 选择：
- `zellij` / `tmux`：write-chars / send-keys 注入（参考 agtalk-office notify.rs，已验证）。
- `gui`：系统通知 + Tauri 弹窗（给人类或带 GUI 的 agent）。
- `webhook:<url>`：POST 回调（给有 HTTP 端点的后台 agent）。
- `none`：不打扰，纯 pull。

### 诚实标注局限

- **"普通终端（无多路复用器）"无标准注入方式**——不假装能解决。文档明确：该环境下 notify 不生效，agent 需自查（`agtalk msg read`）或建议用户在 zellij/tmux 里跑 agent。agtalk-office 对此也无解。
- 注入命令模板末尾（如 `agtalk msg read`）会读 stdin——若 agent pane 当前在交互提示中（sudo 密码/REPL），文本会被当输入。属固有风险，须在用户文档说明。

### 扩展性

- notify 通道用 trait（`NotifyChannel`）抽象，每个通道一个实现。新增通道不改 daemon 核心。
- 外部 notify 命令插件路径必须绝对，参数数组执行（不经 shell）。

---

## 6. 构建与验证命令

```bash
# Rust（-p agtalk 明确指定 src-tauri crate）
cargo check -p agtalk                                        # 快速编译检查
cargo build -p agtalk                                        # debug 构建
cargo build -p agtalk --release --features custom-protocol   # release（直接 cargo build 必须
                                                              # 开 custom-protocol，否则 GUI 白屏）
cargo test -p agtalk -- --test-threads=1                     # 全部测试（推荐单线程，避免状态竞争）
cargo clippy -p agtalk -- -D warnings                        # lint，零警告
cargo fmt --check                                            # 格式检查

# 前端
pnpm install
pnpm typecheck                       # vue-tsc --noEmit
pnpm build                           # typecheck + vite build
pnpm dev                             # Vite dev server

# 扩展
pnpm install                           # 根前端依赖
pnpm typecheck:extension               # cd extension && pnpm run typecheck
pnpm build:extension                   # cd extension && pnpm run build

# 端到端（浏览器扩展身份生命周期）
./scripts/e2e-browser.sh               # 需要 cargo build 生成 debug 二进制

# Tauri dev
pnpm tauri dev -- gui

# Daemon（开发态用 target/debug，安装态用 ~/.local/bin/agtalk）
./target/debug/agtalk daemon start
./target/debug/agtalk daemon status

# 常用 agent 命令
agtalk                                 # agent 最小必读帮助
agtalk --json                          # agent 最小必读帮助（JSON）
agtalk id join <name> --intro ... --workspace ...
agtalk id show
agtalk --as <name> id show             # 指定本地身份
AGTALK_NAME=<name> agtalk id show      # 通过环境变量指定身份
agtalk msg send <uuid> "<body>"
agtalk msg read
agtalk msg wait [msg-id] --timeout <sec>
agtalk mem pack [topic]
agtalk tool doctor
agtalk run [file.yaml]                 # YAML 安全宏编排
```

**提交前必过**：`cargo check` + `cargo test -p agtalk -- --test-threads=1` + `cargo clippy -p agtalk -- -D warnings` + `cargo fmt --check` + `pnpm typecheck` + `pnpm build` + `pnpm typecheck:extension` + `pnpm build:extension`。

---

## 7. CI 要求

CI 已配置在 `.github/workflows/ci.yml`，必须保持绿色。提交前本地先跑齐对应命令：
- `cargo fmt --check`
- `cargo clippy -p agtalk -- -D warnings`
- `cargo test -p agtalk -- --test-threads=1`
- `pnpm build`
- `cd extension && pnpm run typecheck`
- `cd extension && pnpm run build`
- `./scripts/e2e-browser.sh`（e2e job 依赖 rust job 完成后运行）

新增 workflow 或修改 CI 步骤时，需确保本地可复现，避免仅依赖 CI 环境。

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

---

## 12. agent 运行时行为约定（解决"agent 会偷懒"）

这一节是写给**作为 agtalk 客户端的 AI agent** 的行为约定（不是写给 agtalk 本身的开发者）。问题是：agent 的核心循环是"接收消息→调工具→返回"，**不会自发地主动查 agtalk 收件箱**，导致对话中断。解决靠两层配合：

### 12.1 两层配合

```
notify（打扰层，design §5）：daemon 主动敲门 → agent 意识到"有消息"
工作循环检查（本节约定）：  agent 每轮任务后例行查 → agent 真的去取
```

- notify 解决"agent 不知道有消息"。
- 工作循环检查解决"知道了但懒得查"。
- 两者互补：notify 是 daemon 的能力，工作循环检查是 agent 的承诺。

### 12.2 核心约定：每轮任务后必查收件箱

**作为 agtalk 客户端的 agent，在每次完成一轮任务、准备回复用户之前，必须先执行：**

```bash
agtalk msg read
```

行为：
- **有新消息**：先处理该消息（继续对话 / 执行消息里的指令 / 告知用户有待处理事项），再回复用户当前任务。
- **没有新消息**（命令返回错误码 `inbox_empty`）：不阻塞，立即继续回复用户。

**这个检查必须在每轮任务后都执行，不能跳过。** 它是 agent 工作循环的一部分，开销很小（一次秒级命令），但确保 agtalk 消息不会"躺在 daemon 里没人取"。

### 12.3 典型循环

```
agent 标准循环：
  1. 接收用户/上游消息
  2. 调工具完成任务（含可能调 agtalk send 发消息给其他 agent/human）
  3. 【必做】agtalk msg read   ← 检查 agtalk 收件箱
       有新消息 → 处理（可能触发新一轮循环）
       无新消息 → 继续
  4. 回复用户
```

### 12.4 局限（必须知晓）

- **不是实时的**：如果 agent 正在执行一个长任务（如几分钟的代码生成），中途来的 agtalk 消息要等任务完成才被发现。这是不可避免的——agent 不会中断当前任务。
- **约定不是强制**：这是行为约定，依赖 agent 遵守。配合 notify（design §5）的打扰信号，能显著提高 agent 遵守率（notify 让它"意识到该查"，本约定让它"真的查"）。
- **需要时可用 wait 替代**：当 agent 明确在等某条特定回复（如刚发了审批请求等人类响应）且预期 30 秒内有结果，可用 `agtalk wait <msg-id> --timeout 30` 阻塞等，而不是反复 `msg read`。

### 12.5 给 agent 实现者/skill 编写者的指引

- 把"每轮任务后 `agtalk msg read`"写进 agent 的系统提示或 skill（见 `skills/agtalk-bridge/`）。
- 在 agent 的工作循环代码里（若有），把 `msg read` 检查放在"回复用户前"的固定位置。
- 不要依赖 agent"自觉"——把这条作为明确指令写入 prompt/skill，而非含糊建议。
