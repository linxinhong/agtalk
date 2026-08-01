# 图工程大版本评审报告（Tim）

> 评审对象：agtalk 0.2.7 图工程（P0–P3）
> 评审人：专家 Tim（只读抽查，未全读代码）
> 评审依据：`docs/design_graph.md`、`docs/design.md` §11、`docs/release-0.2.7-graph-engineering.md`
> 结论：**基本一致，存在若干明确偏差**（6 项符合、3 项有偏差；5 项已知局限全部属实；另有 4 项未标注偏差、6 项风险）

## 1. 逐项核对结论

| # | 核对项 | 结论 | 关键证据 |
|---|---|---|---|
| 1 | Compiler 四类校验 | ✅ 符合 | `compiler.rs`（环/重复 ID/依赖缺失/契约必填/冲突对/`..` 拒绝） |
| 2 | 状态机与乐观锁 | ✅ 符合 | `state.rs`（can_transition 迁移表、WHERE version=?、收敛规则） |
| 3 | Verification 不 spawn | ✅ 符合 | `verify.rs`（路径/checksum/schema 全进程内）；symlink 未做（安全缺口） |
| 4 | 并行调度 + 关键路径 | ⚠️ 基本符合 | `scheduler.rs`（候选/冲突/权重）；Ready 卡死路径 + 完整优先级未实现 |
| 5 | Worktree 白名单 | ✅ 符合 | `workspace.rs`（参数数组、白名单、提交=交集、merge）；单测全链路 |
| 6 | Approval 复用 | ✅ 符合 | `approval.rs`（approval_request→human→回调 succeeded/failed）；形态差异：通过直接 Succeeded |
| 7 | 恢复与重试 | ⚠️ 基本符合 | `reconciler.rs`（lease→timed_out→重试、脏保留）；探测+grace 未实现 |
| 8 | Graph Patch | ✅ 符合 | `graph.rs` patch_run（仅非 running/terminal）；draft/validating 也允许（瞬时态影响小） |
| 9 | 端到端闭环 | ⚠️ 提交轨迹覆盖 P0–P3；文档表述有夸大 | `.git/logs/HEAD`；e2e 脚本实际走 workspace_skipped 降级，worktree 闭环仅单测覆盖 |

## 2. 未标注偏差（Tim 新发现）

1. **e2e 文档夸大**：`release-0.2.7` 称"端到端真实闭环…worktree→commit→merge"，但 `e2e-graph.sh` 在非 git 目录运行走降级路径；真实 worktree 闭环只在单测验证
2. **派发未触发 notify**：design_graph §5.8 要求"派发 = msg/send + notify 提醒"，`graph_dispatch.rs`/`approval.rs` 只 send 不 notify
3. **Reconciler 无探测/grace**：§5.7-3"发探测消息→grace→timed_out"未实现，daemon 重启可能误判活跃 agent
4. **Capabilities 能力匹配未实现**：spec 的 `capabilities` 字段无消费方（只做在线性检查）
5. **优先级策略不完整**：spec 的 `priority` 字段未被调度消费（仅下游权重排序）

## 3. 风险清单（建议优先级）

| # | 风险 | 证据 | 建议 |
|---|---|---|---|
| ① | **Ready 状态节点永久卡死**：`advance_to_dispatched` 对 Ready 节点执行 `Ready→Ready`，迁移表无此对 → IllegalTransition 放弃，节点不再前进 | `scheduler.rs:368-385` + `state.rs:139-170` | Ready 分支改 `Ready→Leased`，或补 `(Ready,Ready)` |
| ② | **commit 时序悬挂**：节点先 Succeeded 再 commit（`?` 中断请求），commit 失败则图不收敛下游不派发；`git add` 错误被 `let _` 吞 | `graph_verify.rs:190-214`、`workspace.rs:160-162` | commit 放 transition 前，或失败回滚节点 failed |
| ③ | **审批并发窗口**：`dispatch_approval` 的 VersionConflict 被忽略仍继续写 | `approval.rs:69-85` | 非 Applied 结果显式处理 |
| ④ | **Reconciler 误判双跑**：无探测直接 timed_out，300s 租约内长任务重启后被误判重派 | `reconciler.rs:35-55` | 补探测+grace |
| ⑤ | **symlink 逃逸**：字符串级路径验证不解析链接，write_paths 内链接可指向 worktree 外 | `verify.rs:110-128`、`workspace.rs:160-162` | 补 canonicalize 校验 |
| ⑥ | **merge 冲突无指引**：仅标 failed，无冲突事件/恢复指引 | `workspace.rs:211-219` | 追加 workspace_conflict 事件 |

## 4. 总体建议

1. 优先修复风险①（Ready 卡死）与②（commit 时序）
2. 补齐 notify 派发、Reconciler 探测/grace
3. release 文档"端到端真实闭环"表述收敛为"worktree 全链路单测验证 + e2e 主链路验证"，保持文档-实现一致
