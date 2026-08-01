# 图工程端到端测试方案

> 适用版本：agtalk 0.2.7（图工程 P0–P3 + Tim 评审修复 + graph analyze 全量）
> 执行环境：**用户桌面终端**（需能访问 `~/.config/agtalk2` 启动 daemon；本测试不依赖沙箱限制）
> 预估耗时：约 30 分钟（含 D 场景的 6 分钟等待）

---

## 0. 前置检查（5 分钟）

```bash
# ① daemon 版本必须是 0.2.7（图工程端点）
agtalk daemon status        # 显示 version: 0.2.7；若不是则：
# agtalk daemon stop && agtalk daemon start

# ② GUI 二进制必须是 custom-protocol 构建（否则白屏，AGENTS.md §5.5 已知陷阱）
cargo build -p agtalk --release --features custom-protocol
# 或用项目根 Makefile 的 make release

# ③ 测试 spec 已就位（在提交命令所在目录）
ls .agtalk/graph/           # 应有 demo.yaml / demo2.yaml（可自建）

# ④ 注册测试身份（幂等）
agtalk id join alan --intro "e2e 测试执行者"
```

## 1. 冒烟：主链路（自动脚本，2 分钟）

已隔离 daemon（`AGTALK_CONFIG_DIR` 指向 /tmp，不影响现有配置），跑通
submit → 派发 → heartbeat → result → completed 全链路：

```bash
./scripts/e2e-graph.sh
```

**预期**：脚本输出 3 节点全部 `succeeded` + `graph_completed` 事件。
**说明**：脚本在无 git 的隔离目录运行，写节点走 `workspace_skipped` 降级（不建 worktree）——这是设计内行为，不是缺陷。真实 worktree 闭环见场景 B。

## 2. 场景 B：真实闭环（worktree → 写文件 → 验证 → commit → merge）（8 分钟）

**在 agtalk 项目根执行**（本地 git 仓库存在，触发完整 worktree 流程）：

```bash
# ① 提交图（自动探测 repository=/Users/.../agtalk, base=main）
agtalk --as alan graph submit demo
# → 输出 run_id（记下来）

# ② 确认派发 + worktree 创建（等 1-2 秒调度节拍）
agtalk --as alan graph status <run-id>
#   预期：hello 节点 dispatched，且：
ls ~/.agtalk/worktrees/      # 应有 <short-run-id>-hello/ 目录（demo spec 若写节点）
git worktree list            # 能看到该 worktree 分支 agtalk/<run-id>-hello

# ③ 作为 participant 执行节点（模拟 agent 收到派发消息后的动作）
#   在 worktree 内创建/修改文件（write_paths 声明范围内，如 src/demo/hello.rs）
echo 'pub fn hello() {}' > <worktree-path>/src/demo/hello.rs

# ④ 心跳 + 上报结果（result.json 见 docs/graph-participant-protocol.md §6）
agtalk --as alan graph node heartbeat --run <run-id> --node hello --attempt 1
cat > /tmp/result.json <<'EOF'
{ "result": "已完成 hello 实现", "changed_files": ["src/demo/hello.rs"],
  "output_artifacts": [], "verification_claims": [], "blockers": [] }
EOF
agtalk --as alan graph node result --run <run-id> --node hello --attempt 1 --file /tmp/result.json

# ⑤ 验证收敛（等 1-2 秒）
agtalk --as alan graph status <run-id>
#   预期：hello succeeded → 图 completed
git log --oneline -3         # 预期出现 "graph <run-id> hello" 提交
git log --oneline main -3    # 预期该提交已 merge 进 main（场景 B 全部通过的标准）
```

**检查点**：
- [ ] worktree 目录创建于 `<repo>/.agtalk/worktrees/<short-run-id>-<node-key>`
- [ ] 节点 succeeded 且图 completed
- [ ] worktree 分支 commit 含本次改动
- [ ] `main` 头部能看到 merge 结果（`Merge branch 'agtalk/<run-id>-hello'` 或快进）

## 3. 场景 C：审批门禁（human approval 节点）（5 分钟）

```bash
# ① 提交审批 spec（已备好 .agtalk/graph/approval-demo.yaml）
agtalk --as alan graph submit approval-demo

# ② 等节点到达审批点
agtalk --as alan graph status <run-id>
#   预期：approval 节点 waiting_approval（GUI 黄灯）
#   预期：human 收件箱收到审批请求（popup/通知）

# ③ 在 popup 中选择"批准"
# ④ 验证
agtalk --as alan graph status <run-id>
#   预期：approval succeeded → 下游节点继续派发 → 图最终 completed
```

**检查点**：
- [ ] 审批请求到达 human 收件箱（走现有 msg ask 链路，非第二套审批）
- [ ] 批准后节点 waiting_approval → succeeded，下游继续

## 4. 场景 D：恢复——租约探测与超时（6 分钟，含等待）

验证 lease 过期后的"探测 → grace → timed_out"机制（Tim 评审修复项）：

```bash
# ① 提交探测 spec（已备好 .agtalk/graph/probe-demo.yaml；participant=alan 在线但不执行，
#    故意不上报 → lease 过期触发探测机制）
agtalk --as alan graph submit probe-demo

# ② 等 lease 过期（DEFAULT_LEASE_SECONDS = 300s）
sleep 300

# ③ 观察探测消息（daemon reconcile 节拍每 30s 一次）
agtalk --as alan graph logs <run-id>
#   预期：node_probe_sent 事件（lease 过期先探测，不直接判死）

# ④ 再等 grace（PROBE_GRACE_SECONDS = 60s），仍无心跳
sleep 60
agtalk --as alan graph status <run-id>
#   预期：节点 timed_out（若 retryable 且未超 max_attempts → 自动新 attempt 重新派发）
```

**检查点**：
- [ ] `logs` 中先出现 `node_probe_sent`，grace 内节点不立即超时
- [ ] grace 后无心跳 → `timed_out`；retryable 节点自动重派（新 attempt）

> 缩短等待的技巧：把 `DEFAULT_LEASE_SECONDS`/`PROBE_GRACE_SECONDS` 在测试前临时调小再重编译（不推荐正式测试用）。

## 5. 场景 E：GUI 管理界面（5 分钟）

```bash
agtalk graph gui            # 或 GUI 内"图工程"入口（需 0.2.7 daemon）
```

**逐项检查**：
- [ ] 列表页显示历史 GraphRun（含 demo/demo2，状态/时间）
- [ ] 打开一个 completed 图：画布显示节点（着色区分 succeeded/join/approval）+ 有向边
- [ ] 事件日志可见（submit → dispatched → succeeded → graph_completed 序列）
- [ ] **实时性**：再提交一张新图（场景 B 的步骤），GUI 无需刷新看到节点从 dispatched 变 running→succeeded（SSE 推送）
- [ ] 深色模式下文字可读（配色修复项 1d3d7ca）

## 6. 场景 F：analyze 成本决策（2 分钟）

```bash
agtalk --as alan graph analyze demo2         # 预期：✅ 值得上图（验证可自动化）
agtalk --as alan graph analyze tiny-demo      # 已备好：单节点简单 spec → 预期：⚠️ 不值得上图
```

## 7. 回归与收尾

```bash
# ① 全量自动化回归（项目根）
cargo test -p agtalk -- --test-threads=1    # 预期 485 passed
cargo clippy -p agtalk -- -D warnings        # 预期零警告
cargo fmt --check                            # 预期通过

# ② 清理测试残留
agtalk --as alan graph cancel <run-id>      # 未完成的测试图
git worktree prune                          # 清理孤儿 worktree 登记
rm -rf ~/.agtalk/worktrees/                 # 测试产生的 worktree 目录（如不再需要）
```

## 8. 结果记录模板

| 场景 | 结果 | 失败现象/备注 |
|---|---|---|
| 0 前置（daemon 0.2.7 / GUI 构建） | ☐ | |
| A 冒烟 e2e-graph.sh | ☐ | |
| B worktree 真实闭环 | ☐ | |
| C 审批门禁 | ☐ | |
| D 探测/超时恢复 | ☐ | |
| E GUI 管理界面 | ☐ | |
| F analyze | ☐ | |
| G 回归（485/clippy/fmt） | ☐ | |

---

## 已知环境注意

- **沙箱限制**：本 agent 执行环境无法写 `~/.config`，daemon 只能由你在桌面终端启动/切换（场景 0 ①）。GUI 实测同理需桌面环境。
- **worktree 路径**：`<repo>/.agtalk/worktrees/<short-run-id>-<node-key>`（short id，不是全 run-id）。
- **spec 位置**：提交命令所在目录的 `.agtalk/graph/<name>.yaml`（`graph run` 已移除，统一 `submit`）。
