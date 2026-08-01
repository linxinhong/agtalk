# Graph Participant 协议（agent 侧契约）

> 状态：设计定稿（对应 `docs/design_graph.md` §5.8，M1 派发实现时生效）。
> 适用对象：作为 agtalk 客户端的 AI agent（如 Claude Code）以及其 skill 编写者。
> 本文与 `docs/agent-interaction-protocol.md`（对话协议）、`AGENTS.md` §12（工作循环约定）共同生效。

---

## 1. 定位

agtalk 图工程（`docs/design_graph.md`）把任务图（DAG）安全、可恢复地执行完成。**执行者是 agent 本身**——agtalk 不假设能控制 agent 进程，派发靠消息、提醒靠 notify、响应靠 agent 主动上报。

**本协议是"agent 作为图执行者"的唯一契约**。agtalk daemon 侧不实现任何控制 agent 的机制。

## 2. 总体模型

```text
daemon（图运行时）                          agent（执行者）
    │ 派发消息（msg/send，body = NodeRun 上下文） │
    ├─────────────────────────────────────────► 收到 → 执行任务
    │  ← heartbeat（周期上报，可选）              │
    │  ← result（候选结果，完成时上报）           │
    │    （或 blocker：卡住时上报）               │
    │ 验证：daemon 抽查（路径/checksum/schema）   │
    │ 重派（timeout/失败可重试）→ 新 attempt       │
```

核心原则：

> **Agent 只能"上报"，永远不能直接改 NodeRun 状态。** 状态推进只在 daemon 内。
> 你提交的是**候选结果**，不是最终成功——daemon 会验证后才标 succeeded。

## 3. Agent 的义务（必须遵守）

### 3.1 识别派发消息

收到 `agtalk msg read` 返回的消息中，若 `content_type == "graph_dispatch"`（或 body 含 `node_run` 结构，见 §5），则这是一条图工程派发，按本协议执行，而不是普通对话。

### 3.2 执行期间周期性 heartbeat

- 长任务（预计 > 60s）应周期性上报 heartbeat，防止 daemon 超时重派。
- 短任务（秒级）可不报 heartbeat，直接报 result。
- 默认超时由派发消息里的 `timeout_seconds` 给出；超过该时长未上报 → daemon 标 `timed_out` 并按重试策略重派（新 attempt）。

### 3.3 完成时提交候选结果

任务完成（或阶段性完成）后必须上报 result，**不能只回消息说"完成了"**。

### 3.4 卡住时上报 blocker

遇到无法解决的阻塞（需要人类决策、需要其他 agent 信息、环境故障），上报 blocker 而不是静默挂起。daemon 会标 `blocked` 并保留现场，等待重新规划。

### 3.5 照旧执行每轮 msg read

派发消息与普通消息一样躺在收件箱。执行完一轮任务后照旧 `agtalk msg read`（AGENTS.md §12），不要因为"在等图任务"就跳过。

## 4. 上报命令（CLI）

```bash
# heartbeat：续租（可选，长任务用）
agtalk graph node report --run <run-id> --node <node-key> --attempt <n> --heartbeat

# result：提交候选结果（必须）
agtalk graph node report --run <run-id> --node <node-key> --attempt <n> --result <result.json>

# blocker：报告阻塞（可选）
agtalk graph node report --run <run-id> --node <node-key> --attempt <n> --blocker "<原因>"
```

`run-id`、`node-key`、`attempt` 全部来自派发消息，**不需要 agent 记忆**（compact 也不丢——它们在消息 body 里）。

## 5. 派发消息格式

```json
{
  "node_run": {
    "graph_run_id": "uuid",
    "node_key": "impl-backend",
    "attempt": 1,
    "node_type": "executor",
    "goal": "实现后端模块并测试",
    "workspace": { "path": "/abs/path/worktree", "branch": "agtalk/graph-xxx" },
    "read_paths": ["src/backend"],
    "write_paths": ["src/backend"],
    "forbidden_paths": ["src/frontend", "Cargo.lock"],
    "acceptance": [{ "type": "path", "rule": "changed_within_write_paths" }],
    "timeout_seconds": 300,
    "outputs": { "schema": "source-diff", "artifacts": ["backend-diff"] }
  }
}
```

## 6. result.json 结构（候选结果）

```json
{
  "result": "简要结果描述（结构化文本）",
  "changed_files": ["src/backend/api.rs", "src/backend/mod.rs"],
  "output_artifacts": [
    { "artifact_type": "source-diff", "schema_version": "v1", "uri": "file:///abs/path/...", "checksum": "sha256:..." }
  ],
  "verification_claims": [
    { "command": "cargo test -p backend", "exit_code": 0, "stdout_ref": "file:///.../test.log", "summary": "42 passed" }
  ],
  "blockers": []
}
```

字段说明：

| 字段 | 必填 | 说明 |
|---|---|---|
| `result` | 是 | 结果描述；`--json` 友好，供 GUI/日志展示 |
| `changed_files` | 写节点必填 | 相对仓库根的修改文件列表，daemon 会与 write_paths/forbidden_paths 比对 |
| `output_artifacts` | 按派发消息声明 | 节点产出物引用（uri + checksum），daemon 校验存在性与一致性 |
| `verification_claims` | 建议填 | 你执行过的验证（test/build/lint）与退出码，daemon 结构化留证；**P0 信任你的自证，但会抽查**，造假会被发现并记入运行审计 |
| `blockers` | 否 | 未解决的阻塞（填了则节点按 blocked 处理） |

## 7. 超时、重试与 attempt 语义

- **attempt**：同一节点每次执行尝试编号递增（1, 2, 3...）。失败重试时派发消息的 attempt 会 +1。
- **超时**：`timeout_seconds` 到期无 result 且无 heartbeat → `timed_out`。
- **重试**：按节点 `retry_policy.retryable` 分类决定是否重试；`execution_error` / `agent_timeout` 可重试，`path_violation` / `contract_violation` 不重试。
- **上报错了 attempt**：daemon 按幂等键 `graph_run_id:node_key:attempt` 校验，错报会被拒绝——永远以上报命令里带的 attempt 为准，不要猜。

## 8. 失败现场保留

节点失败后，**Workspace、未提交 diff、测试日志、验证记录默认保留**，daemon 不会自动清理。你不需要在失败时手动清理任何东西；也不要删除失败现场（后续诊断依赖它）。

## 9. 安全约束（agent 侧红线）

1. **不直接改 NodeRun 状态**：没有"标记自己成功"的命令，只有上报（heartbeat/result/blocker）。
2. **不绕过验证**：result 里不要声称没跑过的验证；daemon 抽查 diff 与 claims 一致性。
3. **只在派发的 workspace 内写**：禁止写主工作区、禁止写 read_paths 之外、禁止触碰 forbidden_paths（如锁文件、他人目录）。
4. **不做图运行时才做的事**：不创建/删除 worktree、不 merge/rebase/push、不改图定义。
5. **不知道答案就问**：需要人类决策时上报 blocker，或等 daemon 的 Approval Node 派发审批（复用现有 `msg ask` 流程）。

## 10. 与现有工作循环的整合（skill 要点）

在 agent 系统提示 / skill 中加入：

```
agtalk 图工程执行者协议：
- 消息 content_type == "graph_dispatch" = 图节点派发，按 graph-participant-protocol.md 执行
- 执行完必须用 `agtalk graph node report --result` 上报候选结果，不能只回消息
- 长任务周期上报 heartbeat；卡住上报 blocker
- 每轮任务后照旧 `agtalk msg read`，图任务消息躺在普通收件箱
- 只能上报，不能声称成功——成功由 daemon 验证后判定
```

---

## 11. 本协议何时生效

- M0（compiler）：不涉及本协议（无派发）。
- M1（状态机 + 串行调度 + 派发）：派发消息与上报端点实现，本协议生效。
- M2（验证）：daemon 抽查按 §6 字段执行。
- P0 验收：三节点串行 DAG 由 agent 按本协议执行完毕并 completed。

协议的运行时语义以 `docs/design_graph.md`（§5.8、§6、§7）为准；本文是 agent 视角的可执行说明。
