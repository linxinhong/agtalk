# agtalk 0.2.7 图工程大版本总结（评审材料）

> 评审对象：本次大版本（图工程运行时，P0–P3 全部里程碑）。
> 设计真相源：`docs/design_graph.md`（工程落地架构基线）、`docs/design.md` §11（架构红线）。
> 评审方式：专家 Tim 按本材料 + 必要代码抽查，核对**设计与实现一致性**，不全读代码。

## 1. 背景与目标

agtalk 是本地 Agent 对话总线（daemon 唯一真相源 + 三域统一 mailbox/SSE）。
大版本目标：从"多 Agent 对话总线"升级为**图工程协作运行时**——把已定义好的任务图（DAG）安全、并行、可恢复地执行完成。三个基础：
1. Typed Node 契约与 Graph Compiler（静态校验）
2. NodeRun 状态机与确定性 Verification（可证明完成）
3. 基于依赖、写路径和 Workspace 的并行 Scheduler

## 2. 架构核心决策（红线）

- **daemon 是图状态唯一真相源**：Agent 只能"上报"（heartbeat/result/blocker），状态推进只在 daemon 内
- **消息状态 ≠ 节点状态**：Message delivered ≠ Node running；通过 graph_node_assignments 关联
- **P0 不 spawn**：验证 = agent 自报 claims + daemon 进程内抽查（路径/checksum/schema）
- **Approval Node 复用 human approval 仲裁**（不建第二套审批）
- **幂等与恢复**：幂等键 `graph_run_id:node_key:attempt` UNIQUE + 乐观锁 version + GraphEvent 先落库后推 SSE + Reconciliation
- **失败现场保留**：失败/脏 workspace 不自动清理

## 3. 里程碑实现总览

| 阶段 | 内容 | 实现文件 |
|---|---|---|
| **M0** | Graph Spec 解析 + Graph Compiler（结构/契约/并行冲突/安全四类校验）、DB 迁移 V10（7 张表） | `graph/spec.rs` `graph/compiler.rs` `graph/paths.rs` `storage/migrate.rs` |
| **M1** | NodeRun 12 状态机（迁移表+乐观锁+收敛规则）、GraphEvent、串行 Scheduler、7 个 REST 端点、CLI graph 子命令 | `graph/state.rs` `graph/events.rs` `graph/scheduler.rs` `server/handlers/graph*.rs` `cli/graph.rs` |
| **M2** | Verification 抽查（路径 ⊆ write_paths ∩ forbidden=∅ / artifact sha256 / schema）、失败重试（新 attempt）、Reconciler（lease 过期→timed_out） | `graph/verify.rs` `graph/reconciler.rs` `server/handlers/graph_verify.rs` |
| **M3** | SSE GraphEvent 推送（GraphEventHub + `/api/v1/graph/events/stream`，Last-Event-ID 重放） | `transport/graph_hub.rs` |
| **M4** | Tauri 2 + Vue Flow GUI 画布（`?view=graph`，dagre 布局、状态着色、SSE 实时、事件日志） | `src/components/GraphView.vue` `src/lib/graph.ts` `commands.rs` |
| **P1-1** | 并行调度（conflict_pairs 结构化冲突感知）、Join（all_succeeded/all_terminal）、Gate、纯 on_failure 触发 | `graph/compiler.rs` `graph/scheduler.rs` |
| **P1-2** | 多 Worktree 隔离（git worktree 创建/commit/merge，白名单参数数组）、V11、CLI 探测仓库根 | `graph/workspace.rs` |
| **P1-3** | Repair 子图（有修复者不自动重试→修复节点触发→修复成功→原节点重试，仅一轮防无限） | `server/handlers/graph_verify.rs` |
| **P2** | 脏 Workspace 恢复（保留现场+事件）、Graph Patch（paused/ready 可替换定义重新编译） | `graph/reconciler.rs` `server/handlers/graph.rs` |
| **P3** | 关键路径调度（下游权重优先）、GUI 耗时展示 | `graph/scheduler.rs` `GraphView.vue` |

## 4. 关键实现点（供 Tim 抽查）

1. **Compiler 四类校验**：`graph/compiler.rs`（结构：环/重复 ID/依赖缺失/孤立；契约：timeout/workspace/participant/join_policy/approval；并行冲突：write_paths 重叠/共享契约/同 workspace → conflict_pairs；安全：`..` 拒绝/无边界/无限重试）
2. **状态机迁移表**：`graph/state.rs` `can_transition`（谁推进 → 什么迁移合法）；Agent 只上报，transition 由 daemon 调用；乐观锁 `WHERE version=?`
3. **Verification 不 spawn**：`graph/verify.rs`（verify_paths/verify_artifact/verify_schema 全进程内）
4. **并行调度**：`graph/scheduler.rs`（候选收集→关键路径权重排序→冲突感知+配额）
5. **Worktree 白名单**：`graph/workspace.rs`（git 参数数组、禁 push/rebase、`-c` 全局选项跳过、提交白名单路径=changed_files∩write_paths）
6. **审批复用**：`graph/approval.rs`（dispatch 发 approval_request 到 human mailbox → waiting_approval；human reply 回调 → succeeded/failed(approval_rejected)）
7. **恢复**：`graph/reconciler.rs`（lease 过期→timed_out→可重试则新 attempt；脏 workspace 保留）
8. **Patch**：`server/handlers/graph.rs` `patch_run`（仅 paused/ready）
9. **协议四处同改**：proto.rs / handler / CLI 输出 / 测试

## 5. 验证状态

- **477 个测试**（内存 SQLite + tempfile 隔离），`cargo clippy -D warnings` 零警告，fmt 通过
- 端到端真实闭环（非模拟）：三节点并行图 → git worktree 写文件 → 验证 → commit → merge 进 main
- e2e-graph.sh（隔离 daemon 0.2.7）+ CI 已配置
- GUI 画布：`agtalk graph gui`（custom-protocol 构建）实测可见

## 6. 已知局限（诚实标注）

- deterministic 节点无 participant 时降级 blocked（未做 Runtime 自执行）
- 路径验证为字符串级（symlink canonicalize 防护随 worktree 落地后仍未做）
- Gate 条件表达式未评估（gate 只做汇聚/分叉控制）
- Graph Patch 为完整 spec 替换（非增量 patch）
- P3 剩余优化项未做（历史耗时/负载均衡/结果缓存/动态并发）
