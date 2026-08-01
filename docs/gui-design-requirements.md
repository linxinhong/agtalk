# 图工程 GUI · UI 设计要求

> 用途：图工程管理界面（`agtalk graph gui` / `?view=graph`）的 UI 设计单一依据。
> 状态：节点视觉方案已定稿（方案 H 分段式 + Tim 评审 + 用户裁决）；整体界面框架已实现，本文档为设计基线。
> 配套：`docs/design_graph.md`（架构）、`docs/gui-node-visual-design-brief.md`（节点视觉演进记录）。

---

## 1. 定位与设计原则

- **管理视图，不关心 agent 内部**：界面展示"图工程的任务进入、数据返回、等待、提醒"，不展示 agent 的推理过程。
- **只读加载 + 轻操作**：图由 Agent 生成并提交（`agtalk graph run <spec.yaml>`），GUI 只做加载/查看/取消/暂停/恢复/复制接管提示词——**没有"新建图"入口**。
- **一维度一通道**：每个视觉通道只表达一个语义维度，不叠加（历史教训：认领色曾盖掉失败色）。
- **节点自解释**：节点自身携带全部关键信息（文字/图标/边框），不依赖图例（图例已移除——9 行过于复杂）。
- **实时性**：SSE 推送节点/图状态变化，无需手动刷新。

## 2. 界面结构（三区）

```
┌──────────────────────────────────────────────────────────┐
│ header：运行状态筛选 │ 运行选择 │ 刷新 │ 取消/暂停/恢复 │    │
├──────────────────────────────────┬───────────────────────┤
│ 画布（主区，可缩放/平移）          │ 侧栏                   │
│  · 节点 + 有向边（trigger 标签）   │  · 节点详情面板         │
│  · 运行 meta（run id/状态/仓库）   │  · 事件日志             │
└──────────────────────────────────┴───────────────────────┘
```

- **header**：GraphRun 状态筛选（全部/运行中/已完成…）、运行下拉选择、刷新、取消（danger）、运行状态提示（"图由 Agent 生成并提交"）。
- **画布**：Vue Flow，`rankdir=LR`，dagre 自动布局（节点 200×56），缩放 0.2–2，`fit-view-on-init`。
- **侧栏**：选中节点的详情（节点/类型/状态/attempt/执行者+在线徽标/耗时/失败原因）+ 操作（复制接管提示词按钮）+ 事件日志（按时间倒序）。

## 3. 节点视觉规范（定稿）

### 3.1 结构（方案 H 分段式，`SegmentedNode.vue`）

```
┌────┬─────────────────┐
│ ▢  │ node_key        │   左 40px：状态 tint 色块 + 类型图标 + 执行者 emoji 小标识
│ 🦊 │ 执行中 · 已认领  │   右：节点名（粗体 13px）+ 状态·认领（11px 次级色）
└────┴─────────────────┘
```

### 3.2 视觉通道（一维度一通道）

| 通道 | 语义 | 规则 |
|---|---|---|
| **左侧 tint 色块** | 生命周期状态 | 7 组色（见 3.3） |
| **类型 SVG 图标** | 节点类型 | 👤executor / `>_`deterministic / 汇聚join / ◇gate / 盾approval；stroke=currentColor 跟随状态色 |
| **执行者 emoji** | 谁执行 | 右下角小徽标；`djb2(participant) % 10` 选 emoji（🤖🧑💻👩💻🦊🐱🐼🦉🐯👾🐙），**同名同 emoji**；struct 节点无 |
| **边框** | 认领 | 实线=已认领（participant 在线）；灰虚线=未认领；中性=结构节点（join/gate/approval） |
| **`!` 角标** | 未认领警示 | 右上角红圆；仅未认领的执行节点 |
| **第二行文字** | 状态 · 认领 | `执行中 · 已认领 / 未认领 / 结构`（自解释，色盲安全兜底） |
| **pulse 动画** | 活性/等待人类 | running/verifying 蓝呼吸 2s；waiting_approval 黄呼吸 1.6s（更抢眼） |
| **hover title** | 完整描述 | 节点 key / 状态 / 执行者（在线/离线）/ 认领 / 失败原因，多行 |

### 3.3 状态 → 色（12 状态归并 7 组）

| 视觉组 | 覆盖状态 | tint 背景 | 主色 | 文字 |
|---|---|---|---|---|
| 未开始 idle | pending / ready | #F1F5F9 | #94A3B8 | #64748B |
| 进行中 running | leased / dispatched / running / verifying | #EFF6FF | #3B82F6 | #3B82F6 |
| 待审批 waiting | waiting_approval | #FFFBEB | #F59E0B | #B45309 |
| 阻塞 blocked | blocked | #FFF7ED | #EA580C | #C2410C |
| 成功 succeeded | succeeded | #ECFDF5 | #10B981 | #059669 |
| 失败 failed | failed / timed_out | #FEF2F2 | #EF4444 | #EF4444 |
| 已取消 cancelled | cancelled | #F3F4F6 | #9CA3AF | #6B7280 |

深色模式：tint 用 rgba(主色, 0.10–0.12)，文字用亮色（#60A5FA 等）——**CSS 必须拆独立规则**（`.dark` 类 + `@media` 各一份；历史 bug：@media 混入选择器列表致整条规则失效）。

### 3.4 类型图标（20×20，stroke=currentColor）

| 类型 | 图形语义 |
|---|---|
| executor | 人形（头 + 肩） |
| deterministic | 终端 `>_` |
| join | 三线汇聚 + 竖线（汇聚语义） |
| gate | 菱形 outline（decision 惯例） |
| approval | 盾牌 + 对勾（人工审批） |

## 4. 边（Edge）规范

- 有向边（source → target），箭头默认；`on_success` 正常样式，`on_failure` 可区分（如红色/虚线——当前 `agtalk-edge-<trigger>` 类预留）。
- 边上显示 trigger 标签（success/failure）。
- Handle：target=Left / source=Right（8px 圆点，hover 变状态主色）。

## 5. 侧栏详情面板

- **KV 列表**：节点 / 类型 / 状态（色块标）/ attempt / 执行者（+ 在线/离线徽标）/ 耗时 / 失败原因。
- **复制接管提示词按钮**：仅 participant 非空显示；Rust 侧生成文本（`identity/prompt.rs`）+ arboard 写剪贴板；成功"已复制 ✓"、失败红字显示原因（不静默）。
- **事件日志**：时间倒序，事件类型 + payload 摘要；随 SSE 实时追加。

## 6. 交互与反馈

| 交互 | 行为 |
|---|---|
| hover 节点 | title 完整描述 |
| 点击节点 | 侧栏详情 + 画布选中态 |
| 状态变化 | SSE 实时更新：tint/文字/边框/pulse（响应式改 data 字段，不重建节点） |
| 取消运行 | header danger 按钮，二次确认可加（当前直接取消） |
| 加载失败 | 显式错误条（历史教训：静默"暂无 GraphRun"误导——必须有 catch 显示错误） |

## 7. 无障碍与通用约束

- **色盲安全**：颜色不是唯一信息载体——状态有第二行文字、认领有边框+文字、类型有图标+字形。
- 深色模式可读（tokens.css `--text-primary/secondary/tertiary`）。
- 中文 UI（状态文案中文）。
- 零新依赖倾向（图标/头像均为内联 SVG/emoji；剪贴板走 Rust arboard 而非 Web API——WKWebView 限制）。

## 8. 验收标准

- [ ] 7 组状态色 + 文字一眼可读，深色模式正常（`.dark` 与 `@media` 双路径）
- [ ] 认领状态不靠颜色也能读出（边框 + 文字 + `!` 角标）
- [ ] 同 participant 的节点 emoji 一致（同名同像）
- [ ] 边连接正确（Handle Left/Right），trigger 标签可见
- [ ] SSE 更新实时（状态变化无需刷新）
- [ ] 复制提示词按钮：成功反馈 / 失败红字原因
- [ ] typecheck + build 通过；无静默失败路径
