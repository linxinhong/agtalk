<!--
  SegmentedNode.vue
  ─────────────────
  方案 H · 分段式画布节点（agtalk 适配扩展版）
  左侧 40px 状态色块 + 类型图标，右侧文字区
  适用于 agtalk 图工程 GUI (Vue 3 + Vue Flow)

  信息优先级：生命周期状态 > 认领状态 > 节点类型 > 节点名
  视觉通道：
    - 状态色块背景 (浅色 tint) → 生命周期状态（7 组，agtalk 12 状态归并）
    - 虚线/实线边框            → 认领状态（未认领 = 虚线 + '!' 角标）
    - SVG 图标                 → 节点类型（5 种：executor/deterministic/join/gate/approval）
    - 文字                     → 节点名 + 状态文案（中文）
    - pulse 动画               → 活性（进行中）+ 等待人类（待审批更抢眼）
-->

<script setup lang="ts">
import { computed } from 'vue'
import { Handle, Position } from '@vue-flow/core'

/* ── agtalk 12 状态 → 7 视觉组 ── */
type StatusGroup =
  | 'idle' // pending/ready/cancelled：未开始/终止
  | 'running' // leased/dispatched/running/verifying：进行中
  | 'waiting' // waiting_approval：等待人类
  | 'blocked' // blocked：阻塞
  | 'succeeded' // succeeded
  | 'failed' // failed/timed_out
  | 'cancelled'

type NodeType = 'executor' | 'deterministic' | 'join' | 'gate' | 'approval'

interface SegmentedNodeData {
  nodeName: string
  nodeType: NodeType
  statusGroup: StatusGroup
  statusText: string
  claimStatus: 'claimed' | 'unclaimed' | 'struct'
  participantOnline?: boolean
}

// 只消费 data（node 名称/类型/状态/认领）；Handle 是本组件子元素，不需要父 props
const props = defineProps<{ data: SegmentedNodeData }>()

const isUnclaimed = computed(
  () => props.data.claimStatus === 'unclaimed',
)
const isStruct = computed(() => props.data.claimStatus === 'struct')

/* ── SVG 图标（5 种节点类型，stroke=currentColor 继承状态色） ── */
const ICONS: Record<NodeType, string> = {
  executor: `
    <svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"
         width="20" height="20" aria-hidden="true">
      <circle cx="10" cy="7" r="3" stroke="currentColor" stroke-width="1.5"/>
      <path d="M4.5 17c0-3 2.5-5.5 5.5-5.5s5.5 2.5 5.5 5.5"
            stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    </svg>`,
  deterministic: `
    <svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"
         width="20" height="20" aria-hidden="true">
      <path d="M3.5 6.5l4.5 3.5-4.5 3.5" stroke="currentColor" stroke-width="1.5"
            stroke-linecap="round" stroke-linejoin="round"/>
      <path d="M10.5 14.5h6" stroke="currentColor" stroke-width="1.5"
            stroke-linecap="round"/>
    </svg>`,
  join: `
    <svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"
         width="20" height="20" aria-hidden="true">
      <path d="M3 4c0 4.5 3 6 7 6s7-1.5 7-6" stroke="currentColor" stroke-width="1.5"
            stroke-linecap="round"/>
      <path d="M10 10v6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
    </svg>`,
  gate: `
    <svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"
         width="20" height="20" aria-hidden="true">
      <path d="M10 3l6 7-6 7-6-7z" stroke="currentColor" stroke-width="1.5"
            stroke-linejoin="round"/>
    </svg>`,
  approval: `
    <svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"
         width="20" height="20" aria-hidden="true">
      <path d="M10 3l6 2v5c0 3.5-2.5 6-6 7-3.5-1-6-3.5-6-7V5l6-2z"
            stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/>
      <path d="M7.5 10l2 2 3-3.5"
            stroke="currentColor" stroke-width="1.5"
            stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`,
}

const iconSvg = computed(() => ICONS[props.data.nodeType as NodeType] ?? ICONS.executor)

/* ── 动态 class ── */
const nodeClass = computed(() => [
  'segmented-node',
  `status-${props.data.statusGroup}`,
  { unclaimed: isUnclaimed.value, struct: isStruct.value },
])
</script>

<template>
  <div :class="nodeClass">
    <!-- 连接桩 -->
    <Handle type="target" :position="Position.Left" />
    <Handle type="source" :position="Position.Right" />

    <!-- 未认领角标（无执行者/离线且待执行） -->
    <span v-if="isUnclaimed" class="seg-badge" title="未认领：执行者离线/无，节点可能卡住">!</span>

    <!-- 左侧分段：状态色块 + 类型图标 -->
    <div class="seg-left" v-html="iconSvg" />

    <!-- 右侧分段：文字区 -->
    <div class="seg-right">
      <span class="node-name">{{ props.data.nodeName }}</span>
      <span class="node-status">{{ props.data.statusText }}</span>
    </div>
  </div>
</template>

<style scoped>
/* ── 状态色（浅色主题，值与 Ardot 设计稿一致，Bento Neutral） ── */
.segmented-node {
  /* 状态主色 */
  --status-running:   #3B82F6;
  --status-waiting:   #F59E0B;
  --status-blocked:   #EA580C;
  --status-succeeded: #10B981;
  --status-failed:    #EF4444;
  --status-idle:      #94A3B8;
  --status-cancelled: #9CA3AF;

  /* 状态浅色 tint（左侧色块背景） */
  --status-running-bg:   #EFF6FF;
  --status-waiting-bg:   #FFFBEB;
  --status-blocked-bg:   #FFF7ED;
  --status-succeeded-bg: #ECFDF5;
  --status-failed-bg:    #FEF2F2;
  --status-idle-bg:      #F1F5F9;
  --status-cancelled-bg: #F3F4F6;

  /* 中性色 */
  --node-bg:      var(--vf-node-bg, #FFFFFF);
  --node-border:  #E5E7EB;
  --text-primary: #111827;
  --text-running:   var(--status-running);
  --text-waiting:   #B45309;
  --text-blocked:   #C2410C;
  --text-succeeded: #059669;
  --text-failed:    var(--status-failed);
  --text-idle:      #64748B;
  --text-cancelled: #6B7280;

  /* 布局 */
  --node-width: 200px;
  --node-height: 56px;
  --seg-left-w: 40px;
  --radius: 8px;

  position: relative;
  display: flex;
  width: var(--node-width);
  height: var(--node-height);
  background: var(--node-bg);
  border: 1px solid var(--node-border);
  border-radius: var(--radius);
  overflow: hidden;
  font-family: 'Inter', system-ui, -apple-system, sans-serif;
  box-sizing: border-box;
  user-select: none;
}

/* ── 左侧分段 ── */
.seg-left {
  flex-shrink: 0;
  width: var(--seg-left-w);
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--seg-bg, transparent);
  color: var(--seg-color, #6B7280); /* SVG currentColor 继承 */
}
.seg-left :deep(svg) {
  display: block;
}

/* ── 右侧分段 ── */
.seg-right {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
  padding: 8px 10px;
  text-align: left;
}
.node-name {
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.node-status {
  font-size: 11px;
  line-height: 1.3;
}

/* ── 状态变体：色块 tint + 状态文字色 ── */
.status-running { --seg-bg: var(--status-running-bg); --seg-color: var(--status-running); }
.status-running .node-status { color: var(--text-running); }
.status-waiting { --seg-bg: var(--status-waiting-bg); --seg-color: var(--status-waiting); }
.status-waiting .node-status { color: var(--text-waiting); }
.status-blocked { --seg-bg: var(--status-blocked-bg); --seg-color: var(--status-blocked); }
.status-blocked .node-status { color: var(--text-blocked); }
.status-succeeded { --seg-bg: var(--status-succeeded-bg); --seg-color: var(--status-succeeded); }
.status-succeeded .node-status { color: var(--text-succeeded); }
.status-failed { --seg-bg: var(--status-failed-bg); --seg-color: var(--status-failed); }
.status-failed .node-status { color: var(--text-failed); }
.status-idle { --seg-bg: var(--status-idle-bg); --seg-color: var(--status-idle); }
.status-idle .node-status { color: var(--text-idle); }
.status-cancelled { --seg-bg: var(--status-cancelled-bg); --seg-color: var(--status-cancelled); }
.status-cancelled .node-status { color: var(--text-cancelled); }

/* ── 认领状态：未认领 → 虚线边框 + 角标；struct → 中性实线 ── */
.segmented-node.unclaimed {
  border-style: dashed;
  border-color: #94A3B8;
}
.seg-badge {
  position: absolute;
  top: -7px;
  right: -7px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: #DC2626;
  color: #fff;
  font-size: 10px;
  font-weight: 700;
  line-height: 15px;
  text-align: center;
  z-index: 2;
}
.segmented-node.struct {
  border-color: #CBD5E1;
}

/* ── 活性动画：进行中呼吸；待审批更抢眼 ── */
@keyframes seg-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(59, 130, 246, 0.35); }
  50% { box-shadow: 0 0 0 6px rgba(59, 130, 246, 0); }
}
.segmented-node.status-running {
  animation: seg-pulse 2s ease-in-out infinite;
}
@keyframes seg-pulse-wait {
  0%, 100% { box-shadow: 0 0 0 0 rgba(245, 158, 11, 0.45); }
  50% { box-shadow: 0 0 0 6px rgba(245, 158, 11, 0); }
}
.segmented-node.status-waiting {
  animation: seg-pulse-wait 1.6s ease-in-out infinite;
}

/* ── 连接桩 ── */
.segmented-node :deep(.vue-flow__handle) {
  width: 8px;
  height: 8px;
  background: var(--node-border);
  border: 2px solid var(--node-bg);
  border-radius: 50%;
}
.segmented-node :deep(.vue-flow__handle:hover) {
  background: var(--status-running);
}

/* ── 深色主题（Vue Flow .dark 或 prefers-color-scheme） ── */
:where(.dark, .vue-flow-dark) .segmented-node,
@media (prefers-color-scheme: dark) {
  .segmented-node {
    --node-bg:      var(--vf-node-bg, #1A1B1E);
    --node-border:  #2D2E33;
    --text-primary: #E8E9EB;

    --status-running-bg:   rgba(59, 130, 246, 0.12);
    --status-waiting-bg:   rgba(245, 158, 11, 0.12);
    --status-blocked-bg:   rgba(234, 88, 12, 0.12);
    --status-succeeded-bg: rgba(16, 185, 129, 0.12);
    --status-failed-bg:    rgba(239, 68, 68, 0.12);
    --status-idle-bg:      rgba(148, 163, 184, 0.1);
    --status-cancelled-bg: rgba(156, 163, 175, 0.1);

    --text-running:   #60A5FA;
    --text-waiting:   #FBBF24;
    --text-blocked:   #FB923C;
    --text-succeeded: #34D399;
    --text-failed:    #F87171;
    --text-idle:      #94A3B8;
    --text-cancelled: #9CA3AF;
  }
}
</style>
