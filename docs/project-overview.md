# agtalk 项目全景卡

> 侦察日期：2026-07（基于 commit `4cd0256` / v0.2.7 的只读侦察）
> 侦察方式：只读，未修改任何代码；证据均来自实际文件路径，未确认项已显式标注。
> 本文件是仓库现状快照，随架构演进可能过时；架构真相以 `docs/design.md` 与 `AGENTS.md` 为准。

## 1. 项目定位

- **项目名称**：agtalk（v0.2.7，workspace 单版本号统一于 `Cargo.toml:6` / `package.json:4` / `extension/package.json:3`）
- **项目类型**：本地 Agent 对话总线 —— 单二进制 Rust daemon + 薄客户端（CLI / Tauri GUI / WXT 浏览器扩展）
- **目标用户**：本地 CLI agent（Claude Code 等）、人类（审批弹窗/GUI/飞书）、网页 AI（浏览器扩展）
- **核心目标**：三个对话域（agent↔agent、agent↔human、agent↔browser）用**同一套 mailbox + SSE 机制**可靠传话，身份存文件系统、消息先持久化再推送（at-least-once）
- **当前阶段**：核心域已成熟（identity/routing/transport/human/feishu/mem/run 全部实现并有测试）；最近开发方向是 notify 打扰层插件化（cmux/tmux/zellij 插件）与发布运维（release workflow、Windows 兼容）
- **关键架构原则**（`AGENTS.md` 红线，均有代码实现佐证）：路由只认 UUID；name 纯展示；身份载体=`.agtalk/` 文件系统（PID→agents.json→session.json→UUID 认证链）；SSE 唯一推送；三域统一；browser/human 是高熵 token 例外

## 2. 技术栈地图

### 后端（daemon 核心，单一 crate `src-tauri/`）
- **语言**：Rust（edition 2021，`src-tauri/Cargo.toml`），bin `agtalk` + lib `agtalk_app`
- **Web 框架**：axum 0.7（`server/http.rs`），reqwest 0.12（出站，含 gzip/brotli 解码）
- **数据库**：SQLite（rusqlite 0.31 bundled，`storage/mod.rs:22` 单连接 `Arc<Mutex<Connection>>`，WAL，路径 `<config_dir>/agtalk.db`）
- **通信**：REST（newline-JSON 协议 `ServerMsg`）+ SSE（`transport/`，tokio broadcast）
- **异步**：tokio；弹窗子进程 spawn（`human/popup.rs`）；飞书长连接 tokio-tungstenite + 自研 pbbp2 protobuf 帧（`feishu/ws.rs`）
- **CLI 解析**：clap 4 derive；表格输出 comfy-table

### 前端 GUI（薄客户端）
- **框架**：Vue 3.4 + Vite 5 + vue-tsc，Tauri 2 外壳
- **i18n**：vue-i18n（zh-CN / en-US，`src/i18n/`）
- **命令桥**：8 个 `#[tauri::command]`（`commands.rs`），经 reqwest 直连 daemon HTTP（token 不下发前端）
- **capabilities 最小集**：`core:default` + `core:window:allow-close` + `shell:allow-open`（飞书授权链接用，`capabilities/default.json:4-5`）

### 浏览器扩展（agent↔browser 域）
- **框架**：WXT 0.19 + Vue 3 + TypeScript（`extension/wxt.config.ts`，permissions 仅 `storage`）
- **已实现**：background 消息枢纽 + SSE 订阅（fetch+ReadableStream 手动解析）+ popup 收发 UI；1 浏览器 = 1 mailbox 身份
- **未实现（纯预留，代码零占位）**：content script、平台选择器、标签绑定表（tabId↔address）、auto-mode —— 全 `src/` 无相关代码，无 `src/shared/` 目录

### 外部 notify 插件（`plugins/`）
- shell 脚本：`agtalk-notify-cmux` / `agtalk-notify-tmux` / `agtalk-notify-zellij`，遵循通用协议（`discover` stdout JSON / `send` stdin JSON）

## 3. 仓库结构

```
agtalk/
├── src/                    # Tauri GUI 前端（Vue 3）：App.vue（?popup=1 分流）、components/、lib/ipc.ts、i18n/、styles/
├── src-tauri/src/
│   ├── main.rs (347B)      # argv 分派：__popup → run_popup，否则 run_cli（config gui 拦截转 run_gui）
│   ├── lib.rs              # run_gui / run_popup / run_cli 入口；窗口 900×700 / 480×480
│   ├── proto.rs (500行)    # ClientMsg/ServerMsg 协议（serde tag=type, snake_case）
│   ├── cli/                # 子命令分派、HTTP client、YAML runner、输出格式化
│   ├── identity/           # session.json、agents.json、auth、mailbox、relations、browser/human session
│   ├── routing/            # send / lookup / inbox / reply / wait（只认 UUID）
│   ├── transport/          # SSE 流 + SubscriberRegistry + wake
│   ├── server/             # axum 路由、handler（handlers/msg.rs、id.rs、human/、mem.rs）、daemon.rs
│   ├── storage/            # SQLite 句柄 + 迁移（CURRENT_VERSION=9）
│   ├── human/              # approval 仲裁、popup 弹窗、delivery、client
│   ├── feishu/             # 内置 human transport：card/、router/、ws 长连接、dispatch、setup、token
│   ├── mem/                # plan/context/status/entries + pack
│   ├── notify/             # plugin 协议 + 限流触发
│   ├── tool/               # doctor / version / path
│   ├── config.rs / paths.rs / commands.rs / testutil.rs
├── extension/              # WXT 扩展（background + popup）
├── docs/                   # design.md（单一真相源）、commands.md、notify-plugin.md、human-surfaces.md、android-ble-deck.md 等
├── plugins/                # notify 插件 + tests/smoke.sh
├── scripts/e2e-browser.sh  # 浏览器身份生命周期 e2e
├── skills/agtalk-bridge/   # 给 agent 用的 skill
├── .github/workflows/      # ci.yml + release.yml
└── report_work/            # ⚠️ untracked 无关目录（Python 数据报表），未纳入 git
```

## 4. 核心入口

- **二进制入口**：`src-tauri/src/main.rs:8-14` —— `argv[1]=="__popup"` → `run_popup(msg_id)`；否则 `run_cli()`
- **daemon**：`cli daemon` 子命令 → `server/daemon.rs`（127.0.0.1:19527 默认，`daemon.pid`/`daemon.json` 持久化）
- **HTTP API**：`server/http.rs:17-70` axum Router，全部返回 `Json<ServerMsg>`
- **SSE**：`GET /api/v1/events`（`http.rs:695-757`），支持 `Last-Event-ID` 重放
- **GUI**：`lib.rs:27-56` run_gui（900×700 可调）；`lib.rs:61-97` run_popup（480×480 不可调，加载 `index.html?popup=1`）
- **扩展**：`extension/src/entrypoints/background.ts` + `popup/`

## 5. 数据模型地图

### SQLite（`storage/migrate.rs`，CURRENT_VERSION=9，全部有 FK 约束）
| 表 | 用途 |
|---|---|
| `mailboxes` | 邮箱=agent 身份（address PK、name、intro、workspace、left_at、notify_channel、workspace_root） |
| `event_sequences` | 每收件人单调 event_id（SSE 重放基石） |
| `messages` | 核心消息（id PK、to/from_address FK、body、content_type、reply_to_id、metadata、event_id、status，UNIQUE(to_address,event_id)） |
| `message_status_log` | 状态变更审计（pending/delivered/read/done） |
| `browser_sessions` | 浏览器 token（V4） |
| `mem_index` | 记忆在线索引（V5） |
| `human_deliveries` | human 送达状态机（surface、attempts，UNIQUE(message_id,surface)，V9） |
| `human_action_receipts` | 跨端幂等回执（PK(surface,external_event_id)，V9） |
| `approval_resolutions` | 审批首胜仲裁（request_message_id PK，V9） |
| `system_mailboxes` | human 等系统角色 |

### 文件系统身份
- `.agtalk/<name>/session.json`（v2，0600）：`{address, name, intro, notify:{channel, endpoint}}`（`session_file.rs`）
- `.agtalk/agents.json`（0600）：`{anchors: {pid: {name, start_time}}}`（`agents_map.rs`）
- `.agtalk/<name>/relations.json`（0600）：协作画像，**明确不参与认证/路由/SSE/notify**（`relations.rs:3-4`）
- `.agtalk/<name>/memory/`：plan.md + context.md + status.json + entries.jsonl
- `<config_dir>/human/session.json`（0600）：human address + token（`human_session.rs`）
- `<config_dir>/browser/<name>/session.json`：浏览器身份（`browser_session.rs`）

## 6. 接口边界

### REST API（三类认证，`server/handlers/mod.rs:43-78`）
- **agent 认证**：`X-AgTalk-Address` + `X-AgTalk-Pid` + `X-AgTalk-Start-Time` + `X-AgTalk-Workspace-Root`（必带、绝对路径、以 `.agtalk` 结尾）
- **browser**：`X-AgTalk-Address` + `X-AgTalk-Browser-Token`
- **human**：`X-AgTalk-Human-Token`
- **免认证**：id/join、id/lookup、id/cleanup、tool/version、tool/path、config/*、daemon/status、browser/join

端点组：`id/*`（join/leave/cleanup/me/lookup）、`msg/*`（send/reply/done/ask/inbox/read/wait）、`mem/*`（plan/add/search/show/list/pack）、`tool/*`（doctor/version/path）、`config/*`、`daemon/status`、`browser/join`、`human/*`（inbox/read/reply/done/cancel/delivery/ack/agents/send）、`events`（SSE）

### Tauri IPC（8 个命令，`commands.rs`）
`popup_load / popup_reply / popup_done / popup_cancel`（走 `HumanClient`）+ `gui_load_config / gui_set_config / gui_feishu_setup_begin / gui_feishu_setup_poll`

### CLI 子命令（`cli/mod.rs`）
`daemon` / `id`（join/leave/cleanup/show/lookup）/ `msg`（send/reply/done/ask/read/inbox/wait）/ `mem`（plan/entries/pack/relation）/ `run` / `tool`（doctor/version/path）/ `config`（show/get/set/path/gui）/ `help`（`agtalk` 无参输出 AgentHelp 最小必读帮助）

### notify plugin 协议（`notify/plugin.rs`）
`<plugin> discover` → stdout JSON endpoint；`<plugin> send [--dry-run]` → stdin JSON payload（**不含正文/secret**）；路径解析：config → `<config_dir>/plugins/agtalk-notify-<name>` → PATH，拒绝 `..` 逃逸

## 7. 核心业务流程

### 流程 1：消息发送与投递（at-least-once + 断线不丢）
```
agtalk msg send <uuid> → cli/mod.rs → POST /api/v1/msg/send（agent 四 header 认证）
→ handle_send（server/handlers/msg.rs:16）
  → routing/send.rs：校验目标活跃 mailbox（UUID）→ 事务内 event_sequences 自增为接收方分配 event_id → messages 落库 status=pending
  → registry.notify(接收方) → SSE 推送（transport/wake.rs → sse.rs）
  → notify::trigger（限流 1s + 插件打扰）
  → history.jsonl + relations.json 双方更新
  → 若目标是 human mailbox → fanout（popup 弹窗 + feishu 卡片）
接收方：agtalk msg read → lookup::detail_and_mark_read；SSE 断线用 Last-Event-ID 重放（events_since）
```

### 流程 2：Human-in-the-loop 审批（三端仲裁）
```
agent: agtalk msg ask <message> --options ... → POST /api/v1/msg/ask（msg.rs:270）
→ send 落库 → fanout_if_human → human/mod.rs:191
  → popup.rs dispatch：spawn `agtalk __popup <msg-id>`（current_exe() 自定位，480×480 弹窗）
  → feishu/dispatch.rs：审批卡片
人类选择 → POST /api/v1/human/reply（或飞书卡片回调 decide.rs:101）
→ approval::reply（approval.rs:42）单事务 7 步：先写 receipt（幂等）→ 校验选择合法性 → 插回复 → INSERT OR IGNORE approval_resolutions（首胜）→ 原消息 done → commit
→ after_human_reply → SSE 通知 agent + popup.settle / feishu.settle（抢答关窗/改终态卡）
```

### 流程 3：浏览器扩展身份生命周期
```
background 启动 → chrome.storage.local 有 session 则 startSSE（带 Last-Event-ID 断线重连 3s/10s/30s）
首次：popup join → POST /api/v1/browser/join → {address, name, token} → 存 storage.local
发消息：POST /api/v1/msg/send（X-AgTalk-Browser-Token）
注销：POST /api/v1/id/leave
（e2e-browser.sh 验证此链路，固定端口 19528）
```

## 8. 测试与运行命令

- **测试**：约 400 用例，就近原则内嵌模块 `#[cfg(test)]`；独立文件 `routing/tests.rs`、`transport/tests.rs`、`feishu/card/tests.rs`、`feishu/router/decide/tests.rs`；隔离 = 内存 SQLite + `tempfile::TempDir` + `EnvGuard`
- **CI**（`.github/workflows/ci.yml`）：rust（fmt/clippy `-D warnings`/test `--test-threads=1`）+ frontend（pnpm build）+ extension（typecheck+build）+ e2e（e2e-browser.sh）
- **发布**（`release.yml`）：tag 触发，macOS+Windows 双平台 `--release --features custom-protocol`，zip artifact + GitHub Release

```bash
cargo check -p agtalk
cargo test -p agtalk -- --test-threads=1
cargo clippy -p agtalk -- -D warnings && cargo fmt --check
pnpm build && pnpm typecheck:extension && pnpm build:extension
./scripts/e2e-browser.sh
```

## 9. 权限与安全

- **认证链**：PID→agents.json→session.json→UUID，叠加 `validate_pid`（agents.json + sysinfo 系统进程 start_time 双因子，`auth.rs:77-99`）防 PID 复用
- **Token 例外**（仅浏览器/human 域）：browser token 存 `browser_sessions` 表；human token 存 `<config_dir>/human/session.json` 0600，**agent 无任何命令可读**
- **文件权限**：全部敏感文件 0600、目录 0700（`paths.rs` 工具函数，doctor 会校验并告警）
- **SSE 认证**：`http.rs:695-757` 三分支校验；`Last-Event-ID` 只做重放游标
- **workspace 固定**：`X-AgTalk-Workspace-Root` 必须绝对路径且以 `.agtalk` 结尾，禁止回退 daemon 启动目录（`server/handlers/mod.rs:21-39`）
- **notify 安全**：只推信号不推正文（防 shell 注入）；参数数组执行不经 shell；插件路径拒绝 `..`；`validate_plugin_name` 白名单
- **审计**：`message_status_log` 全状态变更记录；审批 `approval_resolutions` 首胜；delivery 状态机
- 唯一 `unsafe`：`macos_dock.rs:31`（objc2 FFI 设 Dock 图标，标准用法）

## 10. 高风险修改区域

1. **8 个文件违反 <800 行红线**（AGENTS.md §3.3 硬约束）：`tool/doctor.rs`（2366）、`server/handlers/msg.rs`（2048，其中约 1300 行是测试）、`server/http.rs`（1676，40 个 handler 平铺未拆）、`server/handlers/id.rs`（1461）、`cli/mod.rs`（1133）、`cli/output.rs`（1071）、`human/approval.rs`（881）、`notify/plugin.rs`（833）—— 最大的结构债，重构需谨慎（拆分属于大动作，先确认范围）
2. **前端/扩展零自动化测试**：GUI 与扩展只有 typecheck+build，SSE 解析、chrome.storage、身份生命周期逻辑无单测兜底
3. **`plugins/tests/smoke.sh` 未纳入 CI**：notify 插件行为只能手动验证
4. **`report_work/` untracked 无关目录**混在仓库根（Python 数据报表），不属于项目
5. **`msg/wait` 服务端未实现**（`msg.rs:454-462` 返回 not_supported）：依赖 CLI 本地 SSE 实现，属于"服务端协议占位"，若浏览器端想 wait 需要先补

## 11. 不确定项

- **未确认**：`android-ble-deck.md`（15KB）描述的 Android BLE 移动端认证/transport（design.md §2.7）在代码中是否有实现 —— 侦察未发现对应模块，疑为设计文档超前于实现
- **未确认**：`config.rs` 中 `notify.plugins` 的完整配置 schema 细节（GUI 配置界面与 CLI `config set` 的边界）
- **未确认**：扩展 popup 中 lookup 后"notify_ready 状态"展示与 daemon 实际探测的一致性（属运行时行为，未实测）
- **未确认**：Windows 平台的 notify 插件与 popup 子进程行为（CI 只验证编译，不跑 e2e）

## 12. 总结判断

agtalk 是一个**结构纪律极强、文档先行、测试覆盖扎实**（约 400 用例，安全模型清晰）的中型 Rust 项目：核心三域（agent/human/browser）在架构上已经真正统一（同一 mailbox+SSE+delivery/receipt/approval 机制），feishu 自研协议与 notify 插件化是亮点。主要债集中在**文件大小红线大量违反**（8 个超 800 行文件）与**前端/扩展零测试**；当前开发节奏是 notify 插件完善 + 发布运维。
