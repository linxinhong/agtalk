# AGTALK 图工程设计稿（落地架构）

> 状态：**工程落地架构基线（已拍板）**。配套文档：`docs/design-review-graph-engine.md`（方案评审）、`docs/design.md`（架构真相源，本稿定稿后需回写）。
> 本文用于后续讨论，**尚未动代码**。

---

## 1. 已确认的约束（来自用户/评审，覆盖原方案）

### 1.1 运行模型：agent 由人工手动拉起

- agtalk 对 agent 只有两种影响力：**notify 打扰 + msg 消息**（AGENTS.md §12"agent 会偷懒"）。
- 因此图工程**不假设 daemon 能控制 agent 进程**。Lease/Heartbeat 从"daemon 强制"弱化为"daemon 请求 + agent 回报 + 超时判定"。
- 派发 = 发消息；提醒 = notify；响应 = agent 主动上报。**daemon 永不主动 kill / 注入 agent 进程**。

### 1.2 安全模型：本机单用户，新端点认证后置

- 新端点全部沿用现有认证链（`authenticate_req`：PID→agents.json→session.json→UUID + `X-AgTalk-Workspace-Root`），**实现时落实，当前不展开设计**。
- 图工程不新增免认证端点；`graph/*` 读取/管理端点额外接受 human token（管理界面用），详见 §6。

### 1.3 GUI 需求：图工程流转管理界面（新）

- 需求：一个界面统一展示——**任务进入、数据返回、等待、提醒**，**不关注 agent 内部**。
- 即：GraphRun 生命周期管理视图（提交、状态、进度、阻塞、审批、取消），节点画布 + 事件日志 + 操作面板。
- **已拍板：独立界面**，与现有提问（popup 审批）功能相互独立：`App.vue` 按 `?view=graph` 分流为独立视图，不新建 Tauri 窗口、不与 config GUI 混排（见 §4.3）。
- 技术选型可行性：**已验证可行**（见 §4）。

---

## 2. 评审结论更新（相对 design-review-graph-engine.md）

| 原评审项 | 更新后结论 |
|---|---|
| §5.1 外部 Agent 执行契约 | **弱化版契约**（§5.8）：不要求 daemon 控制，只要求"派发消息 + 轻量上报 + 超时重派"，notify 兜底提醒。仍必须写进 participant 协议，但不再是"daemon 强制执行" |
| §5.2 确定性验证执行面 | **已拍板：P0 不 spawn**。Runtime 不执行命令，第一版验证 = agent 自报 `verification_claims` + daemon 轻量抽查（路径检查 / artifact checksum / schema，见 §5.8）；命令白名单 exec 推 P1。symlink 防护仍属于路径检查范围 |
| §5.3 并发/恢复正确性 | 不变：乐观锁 + 幂等键 UNIQUE + Lease 过期双跑防护 |
| §2.3 认证链 | 采纳，但**实现时落实**（§1.2），设计不再展开 |
| §4 缺失项 CLI/SSE/路径/信任模型 | 全部补入本文 §6/§7 |
| **新增：GUI 画布** | 原方案完全没有 UI 层。本文 §4 给出 Tauri 2 + Vue Flow 方案 |

---

## 3. 总体架构

```text
┌────────────────────────── 图工程运行时（daemon 内）──────────────────────────┐
│                                                                              │
│  Graph Spec (YAML)                                                           │
│      │ submit                                                                │
│      ▼                                                                       │
│  Graph Compiler ──► compiled_graph(JSON) + parallel_groups + approvals       │
│      │                                                                       │
│      ▼                                                                       │
│  GraphRun 状态机 ──► Scheduler（依赖/冲突/配额/Lease）                         │
│      │                      │                                                │
│      │                      ▼                                                │
│      │              NodeRun 状态机 ──► 派发(msg/send + notify)                │
│      │                      │                │                               │
│      │                      │◄── heartbeat / result（agent 上报）             │
│      │                      ▼                                                │
│      │              Verification Gate（路径/命令/Artifact/Review）             │
│      │                      │                                                │
│      │                      ▼                                                │
│      │              Workspace / Worktree 生命周期（git 白名单操作）            │
│      │                                                                        │
│      └────── GraphEvent（追加式） ──► DB ──► SSE ──► GUI 画布 / CLI           │
└──────────────────────────────────────────────────────────────────────────────┘

外部面：
  CLI：agtalk graph submit/status/logs/cancel/node report
  GUI：Tauri 2 + Vue Flow 画布（human token，SSE 实时刷新）
  Agent：收派发消息 → 干活 → agtalk graph node report 上报
```

---

## 4. GUI 画布可行性评估（Tauri 2 + Vue Flow）

### 4.1 结论：可行，且是 Vue 3 生态最顺路径

| 项 | 评估 |
|---|---|
| 渲染库 | `@vue-flow/core` 1.48.2（MIT，peer `vue ^3.3.0`，与项目 Vue 3.4 兼容）。支持自定义节点、按状态着色、边样式、动画、fitView |
| 自动布局 | `@dagrejs/dagre` 3.0（MIT）DAG 分层布局，与 Vue Flow 官方示例组合成熟 |
| Tauri 2 兼容 | 画布纯前端渲染，无系统 API 依赖；Tauri 侧只需开窗口 + 现有 capabilities（`core:default`），不需要新 capability |
| 数据通道 | daemon REST（初始快照）+ 现有 `/api/v1/events` SSE（实时刷新，human token 分支已实现于 `server/http.rs:704-719`）——**无需新推送通道** |
| 维护状态 | vue-flow 1.48.2 持续发布中，活跃 |

### 4.2 备选（不推荐，记录原因）

- Cytoscape.js：图论展示强，但 Vue 集成与节点自定义交互弱于 Vue Flow。
- 自绘 canvas/SVG：工作量大，DAG 布局/拖拽/缩放都要自己造，违背"薄客户端"。
- LogicFlow / X6：更适合流程图编辑器（拖拽建模），本需求是**运行时状态展示 + 操作**，建模非重点。

### 4.3 画布 UI 设计（第一版）

```
┌──────────────────────────────────────────────────────────┐
│ 顶部：GraphRun 列表 ─ 状态筛选 ─ 新建(submit spec) ─ pause/resume/cancel │
├────────────────────────────────┬─────────────────────────┤
│  Vue Flow 画布                 │  侧边面板（选中节点）      │
│  · 节点：自定义组件按状态着色    │  · 输入/输出 Artifact      │
│    pending 灰 / running 蓝      │  · attempts 与失败原因     │
│    succeeded 绿 / failed 红     │  · verification 明细       │
│    waiting_approval 黄          │  · workspace 路径/branch   │
│    blocked 橙 / cancelled 灰    │  · 相关消息(派发/结果)      │
│  · 边：on_success 实线          │                          │
│        on_failure 红色虚线      │  ┌────────────────────┐   │
│        always 灰色             │  │ 事件日志（追加）      │   │
│  · dagre 自动布局               │  │ GraphEvent 实时滚动  │   │
└────────────────────────────────┴─────────────────────────┘
```

- 页面挂载方式（**已拍板：独立界面**）：`App.vue` 三分支分流——`?popup=1` → `PopupView`（审批提问窗口，现状不动）、`?view=graph` → `GraphView`（图工程管理界面，新增）、默认 → `ConfigView`（配置 GUI）。**不新建 Tauri 窗口、不扩展 capabilities**，图工程界面与提问/配置完全独立。入口：`agtalk config gui` 后点导航，或 `agtalk graph gui <run-id>` 直接拉起 `?view=graph`。
- 实时刷新：GUI 用 human token 订阅 `/api/v1/events`（复用 `server/http.rs` human 分支），GraphEvent 到达 → 更新对应节点状态 / 追加日志 / 高亮边。
- 提醒展示：`waiting_approval` 节点高亮 + 事件日志提示（"等待人工审批"），审批动作复用现有 `/api/v1/human/*`（popup 弹窗 / GUI 内嵌审批卡片均可）。

---

## 5. 工程落地架构

### 5.1 模块划分（新领域 `src-tauri/src/graph/`，遵守 <500 行/文件）

```text
src-tauri/src/graph/
├── mod.rs              # 领域出口：入口函数与类型再导出（<100 行）
├── spec.rs             # Graph Spec 解析（serde_yaml）+ Typed Node 契约结构
├── compiler.rs         # 结构/契约/并行冲突/安全检查 → CompiledGraph
├── state.rs            # NodeRun / GraphRun 状态机（迁移表 + 收敛规则）
├── scheduler.rs        # 就绪节点选择、并发配额、冲突检查、Lease 获取
├── workspace.rs        # Worktree 生命周期（git 白名单封装）
├── artifact.rs         # Artifact 登记/校验（checksum）
├── verify.rs           # Verification Gate（路径 diff / 命令 / schema / artifact）
├── exec.rs             # 确定性命令执行（白名单、超时、输出捕获）—— 安全边界（P1 再做）
├── events.rs           # GraphEvent 落库 + SSE 推送
└── reconciler.rs       # daemon 重启恢复 / Lease 过期 / 超时判定
```

> `exec.rs`（确定性命令执行）**已拍板 P1 再做**：P0 Runtime 不 spawn 任何命令，验证靠 agent 自报 + daemon 轻量抽查（`verify.rs` 内做路径/checksum/schema 检查）。

配套改动（遵守 AGENTS.md 纪律）：
- `storage/migrate.rs`：迁移版本 V10，新增表见 §5.2；若 migrate.rs 膨胀建议拆 `storage/migrations/`。
- `server/handlers/graph.rs`：新端点 handler（按 endpoint 分组，禁止平铺进 http.rs）。
- `proto.rs`：新增 ClientMsg/ServerMsg 变体（§6/§7），遵守四处同改。
- `cli/graph.rs`：CLI 子命令 + 输出格式化。
- `commands.rs`：GUI 画布需要的新 Tauri 命令（若有，仅薄桥转发；预计只需窗口级，画布数据全走 HTTP，命令数 <3）。

### 5.2 数据模型（DB 迁移 V10）

第一版 7 张表（含取舍说明）：

| 表 | 关键字段 | 说明 |
|---|---|---|
| `graph_runs` | id PK, goal, spec_snapshot(JSON), compiled_graph(JSON), status, repository, base_revision, integration_target, created_at, started_at, completed_at, failure_reason | spec 不可变快照 = spec_snapshot；编译结果 = compiled_graph |
| `node_runs` | id PK, graph_run_id FK, node_key, node_type, status, attempt, participant_id, workspace_id, lease_token, lease_expires_at, input_artifact_ids(JSON), output_artifact_ids(JSON), started_at, heartbeat_at, completed_at, failure_type, failure_detail, verification_summary, version | **UNIQUE(graph_run_id, node_key, attempt)** 幂等键；`version` 乐观锁 |
| `artifacts` | id PK, graph_run_id FK, producer_node_run_id FK, artifact_type, schema_version, uri, checksum, metadata(JSON), created_at | 不可变引用，Agent 间传引用不传正文 |
| `workspaces` | id PK, graph_run_id FK, owner_node_run_id FK, repository, base_revision, branch, path, status, dirty, created_at, released_at | 一个写入子图一个 worktree |
| `verifications` | id PK, node_run_id FK, verifier_type, rule, expected, actual, status, evidence_artifact_id FK, started_at, completed_at | 结构化验收记录，不进日志 |
| `graph_events` | id PK 单调(全局), graph_run_id FK, event_type, node_key?, payload(JSON), created_at | 追加式审计 + SSE 重放（Last-Event-ID 用本表 id） |
| `graph_node_assignments` | id PK, node_run_id FK, message_id FK(messages.id), kind(dispatch/heartbeat/result/blocker) | NodeRun ↔ 消息关联（设计稿 §十的关联表） |

**取舍说明（讨论点）**：
1. **Edge 不建独立表**：边是 spec 的静态部分，已含在 `compiled_graph` JSON；运行时"激活后继/Join 汇聚"从 compiled_graph 读取。避免双写不一致。若后续需要"按边查询"再物化。
2. **Lease 不建独立表**：租约字段并入 `node_runs`（lease_token/lease_expires_at）。第一版单租约语义足够，减少 join。
3. **Approval 复用现有机制**：Approval Node 派发 = 现有 `msg/ask` → human mailbox → `approval_resolutions` 仲裁（`human/approval.rs`）。NodeRun 记录 `approval_message_id`（经 `graph_node_assignments` 关联），**不建第二套审批**。

### 5.3 NodeRun 状态机（谁推进 + 迁移表）

```
pending ──(Scheduler: 依赖满足+冲突通过+配额)──► ready
ready   ──(Scheduler: 获取 lease)─────────────► leased
leased  ──(Scheduler: 派发消息落库)────────────► dispatched
dispatched ──(Runtime: 收到首次上报)───────────► running
running ──(Runtime: 收到候选结果)──────────────► verifying
verifying ──(Verifier: 全部通过)───────────────► succeeded
verifying ──(Verifier: 失败)───────────────────► failed
running/leased/dispatched ──(超时器/Reconciler)─► timed_out
running/verifying ──(GraphRun cancelled 级联)──► cancelled
waiting_approval ──(approval_resolutions 落库)─► running 或 failed(approval_rejected)
failed ──(Scheduler: 允许重试的类型+未超次数)──► 新建 attempt（新 node_runs 行）
```

- **Agent 只能"上报"**（heartbeat/result/blocker），**永远不能直接改 NodeRun 状态**——状态推进只在 daemon 内（Scheduler/Runtime/Verifier/Reconciler）。这是可证明完成的基础。
- 非法迁移一律拒绝并记 GraphEvent。
- Approval Node 等待期间状态为 `waiting_approval`（比复用 blocked 清晰，评审 §3.1 的补充）。

### 5.4 GraphRun 收敛规则（每轮调度末尾计算）

| 条件 | GraphRun 终态 |
|---|---|
| 所有节点 terminal 且无 failed/blocked/timed_out | completed |
| 存在 failed/blocked/timed_out 且无活动节点 | failed（failure_reason 聚合失败节点） |
| 存在 waiting_approval 节点 | paused |
| 用户取消 | cancelled |

### 5.5 Scheduler（第一版规则，按序检查）

```
1. 收集 pending/ready 节点
2. 依赖检查：全部上游 terminal（Join 按策略 all_succeeded/all_terminal）
3. 输入 Artifact 检查
4. Gate 条件检查（on_failure/on_blocked/always 下游）
5. 写冲突检查：write_paths 重叠 / 同 workspace 多写者 / 共享契约（迁移、lockfile）串行
6. participant 能力检查（executor_requirements ↔ agent intro/specialties）
7. 并发配额（图级并发 / 单 participant 上限）
8. 获取 Lease（乐观锁 UPDATE ... WHERE version=?）
9. 准备/绑定 Workspace（git worktree 白名单操作）
10. 派发：msg/send + graph_node_assignments + notify 提醒
```

优先级：显式 priority > 阻塞下游数量 > 等待时间 > FIFO（关键路径/历史耗时留 P3，不做）。

### 5.6 失败处理（第一版）

| 失败类型 | 处理 |
|---|---|
| execution_error / agent_timeout | 有限重试（新 attempt，幂等键 +1） |
| test_failure / build_failure | 有限重试 + 保留现场；Repair 子图推 P1 |
| path_violation / contract_violation | 拒绝结果，恢复越界文件，标 failed 不重试 |
| workspace_failure | paused，保留现场，等待人工 |
| merge_conflict | 机械冲突进冲突处理节点（P1）；第一版标 failed 留现场 |
| semantic_blocker | blocked，等待重新规划 |
| approval_rejected | failed（该分支）或 cancel（整图，按 spec 策略） |

重试参数：max_attempts + backoff 全部来自节点契约（spec），无无限重试。

**执行质量字段（Tim 评审 P0 采纳）**：`out_of_scope`（语义级禁止，与 forbidden_paths 互补，渲染进派发 prompt）、`constraints`（通用约束）、`completion_definition`（自然语言完成声明，与 acceptance 互补）、`approval.timeout_action`（审批超时流转 approve/reject，缺省 reject，reconciler 在 lease 过期时执行）。result.json 信封补 `warnings`（半完成留证，node_warnings 事件）。

### 5.7 运行恢复（Reconciliation，P0 基础版）

daemon 启动时对 `status in (running/paused)` 的 GraphRun：
1. 收集 leased/dispatched/running 的 NodeRun；
2. lease 未过期 → 保留原状态，等 agent 回报；
3. lease 过期 → 发探测消息（"还在跑吗？"），grace 周期内无回复 → 标 timed_out；
4. workspace 存在且有未提交 diff → 保留现场，不自动清理；
5. 全部判定结束 → 重派可重试节点 / blocked 其余。

避免：重复执行（幂等键 UNIQUE）、重复建 worktree（workspaces 表先查）、重复合并（状态机限制）。

### 5.8 Participant 契约（弱化版，写进 participant 协议文档）

```
daemon 侧：
  派发 = msg/send（body = NodeRun 上下文：run_id/node_key/attempt/workspace 路径/write_paths/验收规则）
  提醒 = notify::trigger（复用现有打扰层）
  收 heartbeat/result = POST /api/v1/graph/node/... （agent 认证）

agent 侧（写进 skill/agtalk-bridge 扩展）：
  收到派发消息 → 执行 → 期间周期性 `agtalk graph node report --heartbeat`
  完成 → `agtalk graph node report --result <result.json>`（候选结果，非最终成功）
  卡住 → `agtalk graph node report --blocker <原因>`

超时判定：NodeRun.timeout 到期无 result 且 grace 期无 heartbeat → timed_out → 重派或 blocked。
notify 兜底：派发时 notify 提醒；agent 每轮任务后照旧 msg read（AGENTS.md §12）。
```

**P0 验证模型（已拍板：不 spawn）**：

```text
agent 上报 result 时携带：changed_files + output_artifacts(uri/checksum) + verification_claims(自证命令与结果)
daemon 抽查（全部在 daemon 进程内做，不执行任何外部命令）：
  1. 路径检查：读取 repository（git 只读），验证 changed_files ⊆ write_paths 且 ∩ forbidden_paths = ∅
     （symlink 解析后校验，防逃逸）
  2. artifact 检查：uri 存在 + checksum 匹配 + schema_version 正确
  3. schema 检查：result 结构符合节点输出 Schema
  4. claims 记录：verification_claims 结构化落库（verifications 表），不信任但留证
  5. 抽查（可选）：对关键节点，daemon 读取 diff 抽查 changed_files 内容与 claims 一致性
确定性命令（test/build/lint）的真实执行信任 agent 自证，P0 接受此局限（手动拉起模型下风险可控），P1 再引入 exec.rs。
```

---

## 6. 端点设计（全部走现有认证，实现时落实）

| 方法 | 路径 | 用途 | 认证 |
|---|---|---|---|
| POST | `/api/v1/graph/submit` | 提交 spec，触发编译校验（spec 需声明 owner participant） | agent / human（**已拍板：允许 human**） |
| GET | `/api/v1/graph/runs` | GraphRun 列表（状态筛选） | human token / agent |
| GET | `/api/v1/graph/runs/:id` | 详情：compiled_graph + 全部 NodeRun 状态 + artifacts | human token / agent |
| GET | `/api/v1/graph/runs/:id/events?since=` | 事件日志（Last-Event-ID 语义复用 since） | human token / agent |
| POST | `/api/v1/graph/runs/:id/pause` / `resume` / `cancel` | 运行控制 | human token / agent |
| POST | `/api/v1/graph/node/heartbeat` | agent 心跳续租 | agent |
| POST | `/api/v1/graph/node/result` | agent 提交候选结果 | agent |
| GET | `/api/v1/graph/runs/:id/artifacts` | Artifact 列表（引用，不传正文） | human token / agent |
| GET | `/api/v1/graph/runs/:id/workspaces` | workspace 状态 | human token / agent |

SSE：复用 `/api/v1/events`，新增 GraphEvent 事件类型（event_type 前缀 `graph_*`，见 §7），订阅者按需过滤。

## 7. 协议与事件

### ClientMsg 新变体（proto.rs，四处同改）
`GraphSubmit{spec}`、`GraphRuns{status}`、`GraphRunShow{run_id}`、`GraphRunEvents{run_id, since}`、`GraphRunControl{run_id, action}`、`GraphNodeHeartbeat{run_id, node_key, attempt}`、`GraphNodeResult{run_id, node_key, attempt, result, changed_files, output_artifacts, verification_claims, blockers}`

### ServerMsg 新变体
`GraphRunSummary{...}`、`GraphRunDetail{...}`、`GraphEvents{events}`、`GraphEvent{...}`、`GraphNodeReportOk{...}`、`GraphRunControlOk{...}`

### GraphEvent event_type 清单（落库 + SSE）
`graph_created / graph_validated / graph_started / graph_paused / graph_resumed / graph_completed / graph_failed / graph_cancelled`、`node_ready / node_leased / node_dispatched / node_started / node_progress / node_verifying / node_succeeded / node_failed / node_blocked / node_timed_out / node_waiting_approval`、`workspace_created / workspace_dirty / workspace_committed / workspace_merged / workspace_released`、`artifact_created / verification_completed / assignment_dispatched / assignment_result`

### CLI 新子命令
```
agtalk graph submit <spec.yaml>        # 提交时自动探测当前目录 git：repository/base_revision（remote/branch/HEAD）
agtalk graph list [--status]
agtalk graph status <run-id>
agtalk graph logs <run-id> [--since]
agtalk graph cancel <run-id>
agtalk graph node report --run <id> --node <key> --attempt <n> --heartbeat|--result <file>|--blocker <text>
agtalk graph gui [<run-id>]            # 拉起图工程管理界面（?view=graph）
```

**git 探测（已拍板）**：`graph submit` 在 CLI 侧读取当前目录 git 的 remote URL、当前 branch、HEAD commit，填入 GraphRun 的 `repository` / `base_revision` / `integration_target`；不要求 spec 显式声明（spec 声明优先，未声明则用探测值）。

---

## 8. P0 里程碑与验收

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | graph/ 模块骨架 + DB V10 + spec 解析 + Compiler（结构/契约校验） | 三节点 DAG 编译通过/环报错/依赖缺失报错（单测） |
| M1 | NodeRun 状态机 + 串行 Scheduler + 派发（msg/send + notify）+ heartbeat/result 端点 | 三节点串行执行，状态全程可查（内存 SQLite 集成测试） |
| M2 | Verification（路径检查 + artifact checksum + schema + claims 落库，**不 spawn**）+ 失败重试（新 attempt）+ GraphEvent + Reconciliation 基础 | 越界写入被拒；测试失败只重跑失败节点；daemon 重启后运行现场可识别 |
| M3 | CLI graph 子命令 + SSE GraphEvent 推送 | `agtalk graph status` 与 `logs` 端到端可用 |
| M4 | GUI 独立画布视图（`?view=graph`：Vue Flow + dagre + human-token SSE 实时刷新，与提问/配置独立） | 提交一个 DAG，GUI 看到从 submitted → running → completed 全流程，节点着色正确，审批节点黄灯 |

**P0 总验收**：一个三节点串行 DAG（实现→测试→验证）经 `agtalk graph submit` 提交，agent 手动拉起执行并上报，daemon 验证通过后 completed；中途人为制造测试失败，仅重跑失败节点；GUI 画布实时反映全部流转。

P1（并行 fan-out/fan-in、多 worktree、Join/Gate、Repair 子图）、P2（Graph Patch、取消/暂停强化）、P3（关键路径/历史耗时/负载均衡）按原方案优先级顺延。

---

## 9. 决策记录（已拍板）

| # | 问题 | 决定 | 影响章节 |
|---|---|---|---|
| 1 | GUI 挂载方式 | **独立视图 `?view=graph`**，与提问/配置完全独立，不新建窗口 | §1.3 / §4.3 / §8 M4 |
| 2 | 确定性命令执行 | **P0 不 spawn**：agent 自报 + daemon 抽查（路径/checksum/schema/claims 落库）；exec.rs 推 P1 | §2 / §5.1 / §5.8 / §8 M2 |
| 3 | graph/submit 是否允许 human | **允许**，spec 声明 owner participant | §6 |
| 4 | repository/base_revision 来源 | **CLI 提交时自动探测**当前目录 git（remote/branch/HEAD），spec 显式声明优先 | §7 |

### 待定（实现细节，M0 开工前定即可）

- 路径语法（write_paths/forbidden_paths 的相对路径 + glob + symlink 解析规则）；
- `waiting_approval` 新状态 vs 复用 blocked（默认新状态，GUI 黄灯）；
- Edge 物化时机（默认从 compiled_graph 派生，不建表）；
- Lease 字段并入 node_runs（默认并入）。

---

## 10. 下一步建议

1. ✅ 开放问题已拍板（§9），`design_graph.md` 即为工程落地架构基线；
2. 更新 `docs/design.md`（新增图工程章节，回写定稿内容）——AGENTS.md 要求架构变更先改 design.md；
3. 写 `docs/graph-participant-protocol.md`（§5.8 的 agent 侧契约，供 skill/agtalk-bridge 引用）；
4. 从 M0 开始实现，每里程碑按 AGENTS.md 纪律提交（测试就近、文件 <500 行、四处同改）。
