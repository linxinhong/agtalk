---
name: agtalk-review
description: |
  当以下场景发生时加载此 skill：
  - Agent 需要对高风险/模糊决策进行二次确认
  - Agent 完成任务后需要反思沟通质量
  - 需要将协作经验沉淀回 REVIEW.md 本身
  此 skill 是 agtalk-assistant 的扩展机制，需先加载 SKILL.md。
---

# agtalk-review

Review 机制是 agtalk 多 Agent 协作的经验沉淀系统。目标是提高决策质量，并将有价值的协作经验持续写回本文件，使规范随时间自我进化。

> ⚠️ Review 是 Agent 自主行为，不是框架强制流程。Agent 基于自身判断决定是否发起 Review，以及找谁 Review。

Review 分两个阶段：
- **执行中**：遇到高风险/模糊决策时，主动找其他 Agent 确认后再继续
- **完成后**：`agtalk done` 之前，反思本次沟通，将有价值的经验写回本文件

---

## 一、执行中 Review（决策阻断）

### 触发条件

执行任务过程中，满足以下任意一条时，暂停执行，主动发起 Review：

| 条件 | 示例 |
|------|------|
| 决策不可逆 | 删除文件、覆盖数据、重构核心模块 |
| 任务描述存在歧义 | 自己无法判断哪种理解正确 |
| 有多个方案权衡不下 | 需要第二意见帮助决策 |
| 影响范围超出当前模块 | 修改会波及其他 Agent 负责的部分 |

不满足以上任何一条时，**自主决策，不发起 Review**，避免不必要的协作开销。

### 找谁 Review？

没有专职 reviewer，由任务上下文决定：

1. 执行 `agtalk list --capabilities` 查看当前在线 Agent
2. 根据决策类型选择最合适的 Agent：
   - 架构/设计决策 → 优先找 `planner` 或 `architect` role
   - 代码质量决策 → 优先找 `reviewer` role
   - 业务逻辑决策 → 优先找最了解该模块的 Agent
3. 没有明显合适的 Agent 时，选择当前在线且空闲的任意 Agent

### 如何发起 Review？

使用现有 `[TASK]` 前缀发送消息，内容结构如下：

```
[TASK] Review 请求：<决策点简述>
- 背景：<当前任务的上下文>
- 我的方案：<打算怎么做>
- 疑虑点：<具体不确定的地方>
请确认可以继续，或给出修改建议。
```

发送时必须阻塞等待：

```bash
agtalk send <reviewer_agent> "[TASK] Review 请求：..." --wait-done --timeout 120 --notify
```

### 被请求 Review 的 Agent 如何处置？

收到 Review 性质的 `[TASK]` 后：

1. 阅读决策内容和疑虑点
2. 用 `[REPLY]` 给出明确结论：**确认继续** 或 **建议调整**（须说明原因）
3. 执行 `agtalk done <message_id>` 解除发起方阻塞

```bash
# 确认继续
agtalk send <requester> "[REPLY] 方案确认，可以继续。理由：..." --notify
agtalk done <message_id>

# 建议调整
agtalk send <requester> "[REPLY] 建议调整：<具体建议>。原因：..." --notify
agtalk done <message_id>
```

---

## 二、完成后 Review（经验沉淀）

### 触发时机

每次执行 `agtalk done` 之前，做一次简短的自我反思。

### 自检三问

1. **沟通是否顺畅？** 有没有出现歧义、反复确认、信息丢失？
2. **有没有踩到坑？** 哪些地方浪费了时间或走了弯路？
3. **有没有值得固化的规律？** 这次的经验能否提炼成一条通用规则？

三问都没有触发洞察 → 直接 `agtalk done`，不做任何修改。

### 写回本文件的规则

有洞察时，在 `agtalk done` 之前更新本文件（`REVIEW.md`），遵守以下原则：

- ✅ 只写通用规则，不写特定任务的细节
- ✅ 新规则与现有规则冲突时，取更通用的那条，替换旧规则
- ✅ 表达相似意思的规则，合并而非并列
- ❌ 不确定是否通用时，不写入
- ❌ 不修改 `SKILL.md`，经验只沉淀在本文件

---

## 三、完整流程示意

```
收到 [TASK]
    ↓
执行过程中遇到关键决策
    ↓
满足触发条件？
  ├─ 否 → 自主决策，继续执行
  └─ 是 → agtalk list 找合适 reviewer
            ↓
           发送 [TASK] Review 请求 --wait-done
            ↓
           等待 reviewer [REPLY] + done
            ↓
           根据反馈继续 / 调整方案
            ↓
任务完成，自检三问
    ↓
有新洞察？
  ├─ 否 → agtalk done
  └─ 是 → 更新 REVIEW.md → agtalk done
```

---

## 经验积累区

此区域由 Agent 在完成后 Review 时自主维护，记录从实际协作中提炼的通用规则。

<!-- 初始为空，由 Agent 随协作经验逐步填充 -->
