# 图工程方案评审

> 评审对象：《AGTALK 图工程核心改进方案》（约 1190 行设计稿）
> 评审基准：`AGENTS.md` 架构红线、`docs/design.md`、当前代码现状（v0.2.7）
> 评审日期：2026-08-01
> 结论前缀：**方案方向正确，具备落地价值；但存在 3 个必须先想清楚的结构性问题（§5）与若干与现有架构的对齐点（§2）**。

---

## 1. 总体判断

该方案的质量在同类"多 Agent 工程编排"设计中属于上乘：

- 明确区分 **Graph Definition 与 NodeRun**（定义/执行分离），与 agtalk 现有的"身份在文件系统、状态在 daemon"哲学一致；
- 明确 **Message delivered ≠ Node running / Message done ≠ Node succeeded**，划清了消息总线与图运行时的边界，这是最容易踩坑的地方，方案已提前意识到；
- **Verification 与失败分类**（§七、§八）拒绝"Agent 说完成就算完成"，是可证明完成的正确骨架；
- **幂等键 `graph_run_id:node_key:attempt`、Lease、GraphEvent 追加日志、Reconciliation** 构成了可靠的恢复模型。

主要问题不在方向，而在**三个未回答的结构性问题**（§5）和**与现有机制的对齐细节**（§2）。

---

## 2. 与现有架构的对齐评审（AGENTS.md 红线逐条对照）

### 2.1 ✅ 三域统一：图工程不应是第四域

方案把"节点派发"建立在 AGTALK 消息上（§十），复用 agent-agent 域，**没有引入新的对话域**——符合红线"禁止域特例"。

**对齐要求**：NodeRun 的派发消息就是普通 `msg/send`（含 `reply_to` 关联），节点状态变化推送必须走统一 SSE（新增 ServerMsg 变体 + GraphEvent），禁止另建推送通道。

### 2.2 ✅ daemon 唯一真相源：成立

"Graph Runtime 和数据库是节点状态的唯一真相来源"与现状一致。**注意**：`NodeRun` 状态机、`GraphEvent` 是新的真相源，但**不能与 `messages` 表状态混为一谈**——方案已明确，保持。

### 2.3 ⚠️ 认证链必须沿用，不能为图工程开免认证口子

现状认证链：PID → agents.json → session.json → UUID（`identity/auth.rs:26-99`），外加 `X-AgTalk-Workspace-Root` 强制绝对路径且以 `.agtalk` 结尾（`server/handlers/mod.rs:21-39`）。

方案新端点（如"提交候选结果"`node/submit`、"上报进度"）**必须走现有 `authenticate_req`**，并绑定 `graph_run_id + node_key + attempt` 与 participant 身份的一致性校验。设计稿未显式写这一点——补充进 §十二"执行限制"。

### 2.4 ⚠️ 持久化纪律：先持久化再推送

红线 5："消息推送前必须先持久化（at-least-once），event_id 单调，支持 Last-Event-ID 重放。"

GraphEvent 是追加式事件——必须同样遵守：**事件落库（含 event_id）后才向 SSE 广播**。建议 GraphEvent 直接复用 `event_sequences` 机制或采用同构的单调序号表，而不是另起炉灶。

### 2.5 ⚠️ 文件大小红线直接受冲击

AGENTS.md §3.3：单文件 <500 行，绝不超 800。现状已有 8 个文件违反（最大 `tool/doctor.rs` 2366 行）。

图工程新增的领域（compiler / scheduler / state machine / workspace / artifact / verification / reconciler）**很容易再产出 3-5 个 1000+ 行文件**。落地时必须：
- 模块划分先行：`src-tauri/src/graph/{spec.rs, compiler.rs, state.rs, scheduler.rs, workspace.rs, artifact.rs, verify.rs, reconciler.rs, mod.rs}`；
- `storage/migrate.rs` 已 13729 字节（约 190 行但密集），新增 8+ 张表后建议拆 `storage/migrations/` 目录；
- `server/handlers/` 继续按 endpoint 拆文件，禁止在 `http.rs` 平铺。

### 2.6 ⚠️ 协议演进纪律：四处同改

新增 Graph 命令 = 新 ClientMsg/ServerMsg 变体，按 AGENTS.md §4.2 必须四处同改（proto.rs / handler / CLI 输出 / 测试）。方案没有设计 CLI 面——补齐建议见 §4。

### 2.7 ✅ notify 复用（可选加分项）

节点派发可以触发现有 notify 打扰（`notify::trigger`），符合"notify 只推信号不推正文"。这是免费的，方案未提，建议补上。

### 2.8 ✅ Approval Node 必须复用 human approval 仲裁

现状已有成熟的审批仲裁：`human/approval.rs`（单事务 7 步：receipt 幂等 → 校验 → 插回复 → `approval_resolutions` 首胜 → done）+ popup 弹窗 + 飞书卡片三端投递。

**Approval Node 应直接复用这套机制**（`msg/ask` → human mailbox → fanout），图运行时只把 `approval_resolutions` 的结果映射回 NodeRun 状态。**禁止**再建一套审批。设计稿未明确这一点，是重要的复用点。

### 2.9 ✅ mem 复用（可选）

NodeRun 结果摘要可写入现有 mem（`.agtalk/<name>/memory/`），失败模式、历史耗时等 P3 指标数据由 GraphEvent 聚合，不占 mem。

---

## 3. 设计内部一致性评审

### 3.1 ⚠️ 状态机缺少显式迁移表（必须补）

NodeRun 状态：`pending/ready/leased/dispatched/running/verifying/succeeded/failed/blocked/timed_out/cancelled`，GraphRun 状态：`draft/validating/ready/running/paused/completed/failed/cancelled`。

设计稿只列了状态集合，**没定义谁允许推进哪条迁移**。必须补一张迁移表 + "推进者"归属（只允许 Runtime/Scheduler 推进，Agent 只能提交候选结果），非法迁移一律拒绝并记 GraphEvent。这是可证明完成的基础，也是测试的第一批用例。

### 3.2 ⚠️ GraphRun 终态如何由 NodeRun 推导（未定义）

"所有节点 succeeded → completed" 的推导规则没写：有 failed 且无活动节点 → failed？等待审批 → paused？有 blocked 且无活动节点 → blocked？建议定义收敛规则表（可在 Scheduler 每轮末尾计算）。

### 3.3 ⚠️ Repair 子图与 Graph Patch 的边界（需厘清）

§八说"测试失败 → 进入 Repair 子图"，§三说"变更图 → 提交 Graph Patch 重新编译"，P2 才做 Patch。二者关系：Repair 子图是运行时**自动生成**的小图（Diagnose→Repair→Verify），Patch 是**人工/外部**提交的新图——建议明确：第一版 Repair 子图仅限**固定模板**（不可自定义节点逻辑），避免"图里套图"失控。

### 3.4 ⚠️ P0 验收标准与失败处理表不一致

P0 验收："测试失败只重跑失败节点"；但 §八失败表说测试失败进入 Repair 子图，而 Repair 子图在 P1 才实现。**建议**：P0 阶段测试/构建失败统一走"有限重试 + 失败现场保留"，Repair 子图延到 P1，避免 P0 实现两套路径。

### 3.5 ✅ on_failure/on_blocked/always 边 + Join all_terminal 的关系成立

设计合理：`all_succeeded` 严格成功汇聚；`all_terminal` 由下游 Gate 分叉。第一版不需要表达式语言，正确。注意 Compiler 需校验：on_failure 边的下游必须是 Gate/Join(all_terminal)，不能是普通 Executor。

### 3.6 ⚠️ Deterministic Node 的执行面（最大的未决细节，见 §5.2）

§三说 Deterministic Node "由 AGTALK Runtime 执行"，§七的 Command Verification 记录"命令/退出码/输出引用"。**Runtime 怎么执行命令？** 设计稿未写。这是安全与正确性的关键点。

---

## 4. 缺失项（建议补充进方案）

1. **CLI 面**：`agtalk graph submit <spec.yaml>`、`agtalk graph status [run-id]`、`agtalk graph logs <run-id>`、`agtalk graph cancel <run-id>`、`agtalk graph resume <run-id>`。其中 `submit` 是免认证还是 agent 认证需明确（建议 agent 认证 + workspace-root）。
2. **SSE 事件形态**：GraphEvent 如何推送给订阅者（按 participant address 推送哪些事件；订阅者通过现有 `/api/v1/events` 收，无需新通道）。
3. **路径语法定义**：`read_paths/write_paths/forbidden_paths` 的语法（仓库相对路径？glob？`..` 禁止？symlink 是否解析后校验？）。**必须防 symlink 逃逸**（workspace 内 symlink 指向 workspace 外，git diff 检查不到，但写操作可能出界）。
4. **Spec 信任模型**：Graph Spec 由谁提交？是否校验提交者与图内 participant 的一致性？恶意 spec 可声明危险命令。
5. **Git 操作面**：Runtime 的 git 操作（worktree add / commit / merge）白名单与仓库校验（必须是已声明的 repository）；"Agent 不允许 merge/rebase/push"靠什么强制（agent 只在 worktree 内拿不到写权限？还是仅靠事后校验？）。
6. **NodeRun 乐观锁**：现状 Storage 是单 Mutex 连接串行化，但 daemon 内多个异步任务并发推进状态时仍需防双推进——建议 NodeRun 加 `version` 字段做乐观锁（UPDATE ... WHERE version=?），与现有 `event_sequences` 思路一致。
7. **heartbeat 的传输**：外部自治 Agent 通过什么上报 heartbeat？（`msg/send` 太重；建议轻量端点 `/api/v1/graph/node/heartbeat` 或复用 msg 但专门化）。见 §5.1。
8. **审计联动**：NodeRun 状态变化写 `message_status_log` 同构的审计（或直接用 GraphEvent 承担，需明确）。

---

## 5. 三个必须先想清楚的结构性问题

### 5.1 外部自治 Agent 的执行契约（影响整个运行时设计）

现状模型：**agent 是外部自治进程**，daemon 无法控制它——daemon 发消息靠 notify 打扰 + agent 自觉 `msg read`（AGENTS.md §12"agent 会偷懒"问题）。而图工程要求：daemon 租约（Lease）、派发、要求 heartbeat、超时重派、验证结果。**这是两种不同的信任模型，方案没有正面回答。**

必须定义 Participant 侧契约，例如（建议写进 skill/协议文档，类似 `agent-interaction-protocol.md` 的扩展）：

- agent 收到 NodeRun 派发消息后，必须用 `agtalk graph node report --run <id> --node <key> --status running/heartbeat/result` 上报；
- 不上报 = 无 heartbeat = Lease 过期 = 超时分类（`agent_timeout`）→ 恢复或重派；
- **派发消息本身带 NodeRun 上下文**（workspace 路径、write_paths、验收规则），agent 不需要记忆，compact 不丢。

不解决这个问题，"Lease/Heartbeat/恢复"就只是纸面机制——**这是 P0 前必须先定的协议**，建议先写 `docs/graph-participant-protocol.md`。

### 5.2 确定性验证的执行面（安全边界）

"Runtime 执行 test/build/lint/typecheck + git diff 检查"意味着 daemon 直接 spawn 子进程跑命令、跑 git。这与现有安全纪律（notify 插件"参数数组执行不经 shell"、路径拒绝 `..`）需要同样的严谨：

- 命令从 spec 来 → **命令必须白名单化**（Deterministic Node 只允许声明过的命令模板，不允许自由 shell）；
- 执行工作目录 = worktree，环境变量最小化；
- diff 检查必须解析 symlink 后校验路径（防 symlink 逃逸）；
- **git 操作只允许白名单子命令**（worktree add/commit/merge/diff，禁止 push/rebase/checkout 分支切换）。

建议 P0 就引入一个 `graph/exec.rs` 子进程执行封装（超时、输出捕获、退出码），与 notify plugin 的超时/输出处理共用模式，**不共用代码**（领域不同，避免耦合）。

### 5.3 并发与恢复的正确性细节（实现量大，需提前定）

- **乐观锁**（见 §4.6）；
- **幂等键** `graph_run_id:node_key:attempt` 落地为 DB UNIQUE 约束，重派前先查；
- **Reconciliation 的判定**：Lease 过期 vs participant 在线（`lookup`/SSE 活跃）的组合要定义清楚——Lease 过期但 participant 在跑，重派会双跑；建议"Lease 过期 → 先发探测消息，等一个 grace 周期，再决定重派"；
- **GraphRun 级暂停/恢复**：paused 状态下已 leased 的节点如何处理（收回？等返回？）。

这些细节不阻塞方案评审，但必须在实现前写成状态机测试用例（内存 SQLite + tempfile 隔离，沿用现有测试纪律）。

---

## 6. 落地建议（P0 收缩版）

方案 P0（最小可靠闭环）方向正确，建议进一步收缩与明确：

| 项 | 方案 P0 | 评审建议 |
|---|---|---|
| 节点类型 | 五类 | P0 只实现 Executor + Deterministic + Approval；Join/Gate 推到 P1（P1 验收才需要它们） |
| Scheduler | 串行 | 串行 + 简单优先级（FIFO 即可），冲突检测仅做 write_paths 重叠检查 |
| Workspace | 绑定 | P0 单 Worktree 策略（整图一个），多 Worktree 推 P1 |
| 失败处理 | 重试 | P0 统一"有限重试 + 保留现场"，Repair 子图推 P1（修正 §3.4 的不一致） |
| 恢复 | — | P0 就要 Reconciliation 基础版（daemon 重启 → 状态收敛，不重派未完成节点即记 blocked） |
| 指标 | — | P0 起就落 GraphEvent，指标全从事件聚合，不另建表 |

**建议的 P0 交付物**（可独立验证）：

1. `graph/` 模块 + DB 迁移 V10（graph_runs / node_runs / graph_events / artifacts / workspaces 五张表起步）；
2. Graph Compiler（结构校验：环、依赖存在、ID 唯一、契约必填）；
3. 串行 Scheduler + NodeRun 状态机（含迁移表测试）；
4. Executor 派发（复用 msg/send）+ Deterministic 命令执行（白名单）+ 路径验证（git diff 检查）；
5. Approval Node 复用现有 approval 仲裁；
6. CLI `graph submit/status/logs/cancel` + SSE GraphEvent 推送；
7. 三节点串行 DAG e2e 测试（沿用内存 SQLite + tempfile）。

## 7. 评审结论

- **方向**：采纳。图工程与 agtalk"daemon 唯一真相源 + 三域统一 + 可证明完成"的哲学完全兼容，且能显著提升多 Agent 协作的确定性。
- **先决条件**：落地前必须先定 **Participant 执行契约（§5.1）**——这是整个运行时成立的前提；其次补 **路径语法 + symlink 防护 + 命令白名单（§5.2）**。
- **必须避免**：为图工程开新的认证免检口子；新建第二套审批；把 NodeRun 状态塞进 messages metadata（方案已明确反对，保持）。
- **架构流程**：按 AGENTS.md 纪律，本方案定稿后应先更新 `docs/design.md`（新增"图工程"章节）再动代码；CLI/协议变更遵守四处同改。

**一句话**：方案可以作为图工程的架构蓝本进入设计定稿，但请先回答 §5 的三个结构性问题，并把 §2.3/§2.4/§2.8 三个对齐点写进设计稿。
