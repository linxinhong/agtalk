<script setup lang="ts">
/**
 * SegmentedNode · 定稿方案 A（分段式 + 像素头像）
 * Vue Flow 自定义节点：<VueFlow :node-types="{ agtalk: SegmentedNode }">
 *
 * data 字段（SSE 更新只改 data，不重建节点）：
 *   nodeKey      节点名
 *   nodeType     executor / deterministic / join / gate / approval
 *   status       12 态之一（内部归并 7 组）
 *   participant  执行者名（可空）
 *   online       执行者在线（默认 true）
 *   attempt      第几次尝试（>1 显示角标）
 *   duration     已运行时长文本，如 "12:03"（可空）
 *   failureReason 失败原因（进 hover title）
 */
import { computed } from 'vue';
import { Handle, Position } from '@vue-flow/core';
import { avatarFor, statusGroupOf, STATUS_TEXT, STRUCT_TYPES } from '../lib/identity'

interface SegNodeData {
  runId?: string
  nodeKey: string
  nodeType: string
  status: string
  group?: string
  participant?: string | null
  online?: boolean
  attempt?: number
  duration?: string
  failureReason?: string
}

const props = defineProps<{ data: SegNodeData; selected?: boolean }>()

const isStruct = computed(() => STRUCT_TYPES.includes(props.data.nodeType));
const group = computed(() => statusGroupOf(props.data.status));
const claimed = computed(() => !!props.data.participant && props.data.online !== false);
const avatar = computed(() => avatarFor(props.data.participant));
const alive = computed(() => group.value === 'running' || group.value === 'waiting');
const showWarn = computed(() =>
  !isStruct.value && !claimed.value && !['succeeded', 'cancelled'].includes(group.value));

const statusText = computed(() => STATUS_TEXT[group.value]);
const claimText = computed(() =>
  isStruct.value ? '结构' : claimed.value ? '已认领' : '未认领');

const title = computed(() => [
  `${props.data.nodeKey} · ${props.data.nodeType} · ${statusText.value}${props.data.duration ? ' ' + props.data.duration : ''}`,
  props.data.participant
    ? `执行者 ${props.data.participant}（${props.data.online !== false ? '在线' : '离线'}）· ${claimText.value}`
    : claimText.value === '未认领' ? '未认领：无执行者在线' : null,
  (props.data.attempt ?? 0) > 1 ? `attempt ${props.data.attempt}` : null,
  props.data.failureReason ? `失败原因：${props.data.failureReason}` : null,
].filter(Boolean).join('\n'));

/* 类型几何字形 ○▢▷◇⬡（stroke=currentColor，跟随状态色） */
const GLYPHS: Record<string, string> = {
  executor:     '<circle cx="8" cy="8" r="5.6" fill="none" stroke="currentColor" stroke-width="1.8"/><circle cx="8" cy="8" r="1.6" fill="currentColor"/>',
  deterministic:'<rect x="3" y="3" width="10" height="10" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.8"/>',
  join:         '<path d="M4 3 L12.5 8 L4 13 Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>',
  gate:         '<path d="M8 2.5 L13.5 8 L8 13.5 L2.5 8 Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>',
  approval:     '<path d="M8 1.8 L13 4.3 V8 C13 11.2 10.9 13.4 8 14.2 C5.1 13.4 3 11.2 3 8 V4.3 Z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/><path d="M5.8 7.8 L7.4 9.4 L10.3 6.3" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>',
};
const glyph = computed(() => GLYPHS[props.data.nodeType] || GLYPHS.executor);
</script>

<template>
  <div
    class="gn"
    :class="[
      `gn--${group}`,
      isStruct ? 'gn--struct' : claimed ? 'gn--claimed' : 'gn--unclaimed',
      { 'gn--alive': alive, 'gn--selected': selected },
    ]"
    :title="title"
  >
    <span class="gn-stripe"></span>

    <!-- 通道 3：执行者（结构节点改放几何字形块） -->
    <span v-if="isStruct" class="gn-shape">
      <svg viewBox="0 0 16 16" v-html="glyph"></svg>
    </span>
    <span v-else-if="avatar" class="gn-avatar">
      <img :src="avatar" :alt="data.participant ?? ''" draggable="false">
      <i class="gn-presence" :class="{ 'gn-presence--on': data.online !== false }"></i>
    </span>
    <span v-else class="gn-avatar--empty">?</span>

    <span class="gn-main">
      <span class="gn-key">{{ data.nodeKey }}</span>
      <span class="gn-sub">
        <b>{{ statusText }}</b><span v-if="data.duration" class="gn-dur"> {{ data.duration }}</span>
        · {{ claimText }}
      </span>
    </span>

    <span class="gn-side">
      <svg v-if="!isStruct" class="gn-type" viewBox="0 0 16 16" v-html="glyph"></svg>
      <span v-if="(data.attempt ?? 0) > 1" class="gn-attempt">×{{ data.attempt }}</span>
    </span>

    <span v-if="showWarn" class="gn-warn">!</span>

    <Handle type="target" :position="Position.Left" class="gn-handle" />
    <Handle type="source" :position="Position.Right" class="gn-handle" />
  </div>
</template>

<style scoped>
/* 与 styles/node.css 一一对应；颜色全部走 tokens.css 变量，
   组件内用具体 CSS 属性渲染（历史坑：自定义节点不消费 --vf-node-bg） */
.gn {
  position: relative;
  display: flex; align-items: center; gap: 8px;
  width: var(--gn-w, 216px); height: 64px;
  padding: 0 10px 0 14px; box-sizing: border-box;
  background: var(--surface);
  border: 1.5px solid var(--border-strong);
  border-radius: var(--r-node, 8px);
  font-family: var(--font-ui);
  cursor: pointer; user-select: none;
  transition: transform 150ms cubic-bezier(.4,0,.2,1),
              box-shadow 150ms cubic-bezier(.4,0,.2,1),
              border-color 150ms cubic-bezier(.4,0,.2,1);
}
.gn:hover { transform: translateY(-1px); box-shadow: var(--shadow-pop); }
.gn--selected { outline: 2px solid var(--ink); outline-offset: 2px; }

/* 通道 1：状态（左色条 + tint + 状态文字） */
.gn-stripe {
  position: absolute; left: 0; top: 0; bottom: 0; width: 4px;
  border-radius: var(--r-node, 8px) 0 0 var(--r-node, 8px);
}
.gn--idle      { background: var(--st-idle-tint); }      .gn--idle .gn-stripe      { background: var(--st-idle-main); }
.gn--running   { background: var(--st-running-tint); }   .gn--running .gn-stripe   { background: var(--st-running-main); }
.gn--waiting   { background: var(--st-waiting-tint); }   .gn--waiting .gn-stripe   { background: var(--st-waiting-main); }
.gn--blocked   { background: var(--st-blocked-tint); }   .gn--blocked .gn-stripe   { background: var(--st-blocked-main); }
.gn--succeeded { background: var(--st-succeeded-tint); } .gn--succeeded .gn-stripe { background: var(--st-succeeded-main); }
.gn--failed    { background: var(--st-failed-tint); }    .gn--failed .gn-stripe    { background: var(--st-failed-main); }
.gn--cancelled { background: var(--st-cancelled-tint); } .gn--cancelled .gn-stripe { background: var(--st-cancelled-main); }

.gn--idle .gn-sub b      { color: var(--st-idle-deep); }
.gn--running .gn-sub b   { color: var(--st-running-deep); }
.gn--waiting .gn-sub b   { color: var(--st-waiting-deep); }
.gn--blocked .gn-sub b   { color: var(--st-blocked-deep); }
.gn--succeeded .gn-sub b { color: var(--st-succeeded-deep); }
.gn--failed .gn-sub b    { color: var(--st-failed-deep); }
.gn--cancelled .gn-sub b { color: var(--st-cancelled-deep); }

.gn--running .gn-type   { color: var(--st-running-main); }
.gn--waiting .gn-type   { color: var(--st-waiting-main); }
.gn--blocked .gn-type   { color: var(--st-blocked-main); }
.gn--succeeded .gn-type { color: var(--st-succeeded-main); }
.gn--failed .gn-type    { color: var(--st-failed-main); }
.gn--idle .gn-type, .gn--cancelled .gn-type { color: var(--st-idle-main); }

/* 通道 2：认领（边框 + `!` 角标） */
.gn--claimed   { border-style: solid; }
.gn--unclaimed { border-style: dashed; }
.gn--unclaimed .gn-avatar img { filter: grayscale(1); opacity: .55; }
.gn-warn {
  position: absolute; top: -7px; right: -7px;
  width: 16px; height: 16px; border-radius: 50%;
  background: var(--claim-warn); color: #fff;
  font-size: 10px; font-weight: 700; line-height: 16px; text-align: center;
  box-shadow: 0 0 0 2px var(--surface);
}

/* 通道 3：执行者（像素头像 + 在线圆点） */
.gn-avatar { position: relative; flex: 0 0 36px; width: 36px; height: 36px; }
.gn-avatar img {
  width: 36px; height: 36px; border-radius: 50%;
  image-rendering: pixelated;
  box-shadow: 0 0 0 2px var(--surface), 0 0 0 3px var(--border);
  background: var(--surface-2);
}
.gn-presence {
  position: absolute; right: -1px; bottom: -1px;
  width: 10px; height: 10px; border-radius: 50%;
  box-shadow: 0 0 0 2px var(--surface);
  background: var(--claim-offline);
}
.gn-presence--on { background: var(--claim-online); }
.gn-avatar--empty {
  width: 36px; height: 36px; border-radius: 50%; flex: 0 0 36px;
  border: 1.5px dashed var(--border-strong);
  color: var(--text-tertiary);
  display: flex; align-items: center; justify-content: center;
  font-size: 14px; font-weight: 600;
}

/* 文字区 */
.gn-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.gn-key {
  font-size: var(--fs-node-key, 13px); font-weight: 600;
  color: var(--text-primary); line-height: 1.2;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.gn-sub {
  font-size: var(--fs-meta, 11px); color: var(--text-secondary);
  line-height: 1.2; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.gn-sub b { font-weight: 600; }
.gn-dur { font-family: var(--font-mono); font-size: 10px; }

/* 通道 4：类型字形 + attempt */
.gn-side { flex: 0 0 auto; display: flex; flex-direction: column; align-items: center; gap: 4px; }
.gn-type { width: 15px; height: 15px; }
.gn-attempt {
  font-family: var(--font-mono); font-size: 10px;
  color: var(--text-tertiary);
  background: var(--surface-2); border: 1px solid var(--border);
  border-radius: 999px; padding: 0 5px; line-height: 14px;
}

/* 结构节点 */
.gn--struct { background: var(--surface); }
.gn--struct .gn-stripe { background: var(--border-strong); }
.gn-shape {
  flex: 0 0 36px; width: 36px; height: 36px;
  display: flex; align-items: center; justify-content: center;
  color: var(--text-secondary);
}
.gn-shape svg { width: 22px; height: 22px; }
.gn--struct.gn--waiting { background: var(--st-waiting-tint); }
.gn--struct.gn--waiting .gn-stripe { background: var(--st-waiting-main); }
.gn--struct.gn--waiting .gn-shape { color: var(--st-waiting-main); }
.gn--struct.gn--succeeded .gn-stripe { background: var(--st-succeeded-main); }
.gn--struct.gn--succeeded .gn-shape { color: var(--st-succeeded-main); }

/* 通道 6：呼吸（蓝缓 / 黄急） */
@keyframes gn-pulse-running {
  0%, 100% { box-shadow: 0 0 0 0 rgba(37,99,235,0); }
  50%      { box-shadow: 0 0 0 4px rgba(37,99,235,.18); }
}
@keyframes gn-pulse-waiting {
  0%, 100% { box-shadow: 0 0 0 0 rgba(217,119,6,0); }
  50%      { box-shadow: 0 0 0 5px rgba(217,119,6,.28); }
}
.gn--running.gn--alive { animation: gn-pulse-running 2s cubic-bezier(.4,0,.2,1) infinite; }
.gn--waiting.gn--alive { animation: gn-pulse-waiting 1.2s cubic-bezier(.4,0,.2,1) infinite; }

/* Handle：8px 圆点，hover 变状态主色 */
.gn-handle {
  width: 8px; height: 8px;
  background: var(--surface); border: 1.5px solid var(--border-strong);
}
.gn-handle:hover { border-color: currentColor; background: currentColor; }
</style>
