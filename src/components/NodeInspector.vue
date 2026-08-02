<script setup lang="ts">
/**
 * NodeInspector · 右侧检查器：节点详情 / 事件日志 双标签
 * 复制接管提示词：成功显式反馈「已复制 ✓」，失败红字原因（不静默）。
 * Tauri 环境走 Rust arboard（invoke('copy_takeover_prompt')），
 * 纯 Web 环境退化 navigator.clipboard。
 *
 * props.node: 选中节点 data（可空）; props.events: [{ time, status, message }]
 */
import { computed, ref } from 'vue';
import { avatarFor, STATUS_TEXT, statusGroupOf } from '../lib/identity'
import { nodePrompt } from '../lib/graph'

interface InspNode {
  runId?: string
  nodeKey?: string
  nodeType?: string
  status?: string
  group?: string
  participant?: string | null
  online?: boolean
  attempt?: number
  duration?: string
  failureReason?: string
}
interface InspEvent {
  id: number
  event_type: string
  node_key: string | null
  payload: Record<string, unknown>
  created_at: number
}
const props = defineProps<{ node: InspNode | null; events: InspEvent[] }>();

const tab = ref('detail');
const avatar = computed(() => avatarFor(props.node?.participant));

/* 复制接管提示词：显式反馈，失败不静默 */
const copyState = ref(''); // '' | 'ok' | 'err:...'
/** 事件 → 状态组（node_xxx → xxx；graph_xxx → idle） */
function evtGroup(eventType: string): string {
  return statusGroupOf(eventType.startsWith('node_') ? eventType.slice(5) : '')
}
function evtText(e: InspEvent): string {
  const brief = JSON.stringify(e.payload)
  const p = brief && brief.length > 60 ? brief.slice(0, 60) + '…' : (brief || '')
  return `${e.event_type}${e.node_key ? ' ' + e.node_key : ''}${p ? ' · ' + p : ''}`
}

async function copyTakeover() {
  copyState.value = '';
  try {
    // Rust 侧生成接管文本 + arboard 写剪贴板（gui_node_prompt，commands.rs）
    const { runId, nodeKey } = props.node ?? {}
    await nodePrompt(runId ?? '', nodeKey ?? '')
    copyState.value = 'ok';
    setTimeout(() => (copyState.value = ''), 2000);
  } catch (e) {
    copyState.value = `err:${e instanceof Error ? e.message : String(e)}`;
  }
}
</script>

<template>
  <aside class="insp">
    <div class="tabs">
      <button :class="{ on: tab === 'detail' }" @click="tab = 'detail'">节点详情</button>
      <button :class="{ on: tab === 'events' }" @click="tab = 'events'">事件日志</button>
    </div>

    <div v-show="tab === 'detail'" class="body">
      <template v-if="node">
        <div class="kv">
          <div class="row"><span class="k">节点</span><span class="v mono">{{ node.nodeKey }}</span></div>
          <div class="row"><span class="k">类型</span><span class="v">{{ node.nodeType }}</span></div>
          <div class="row">
            <span class="k">状态</span>
            <span class="v">
              <span class="pill" :class="`pill--${node.group}`">
                <i></i>{{ STATUS_TEXT[node.group ?? 'idle'] }}
              </span>
            </span>
          </div>
          <div class="row"><span class="k">attempt</span><span class="v mono">{{ node.attempt || 1 }}</span></div>
          <div v-if="node.participant" class="row">
            <span class="k">执行者</span>
            <span class="v">
              <img v-if="avatar" class="ava" :src="avatar" :alt="node.participant">
              {{ node.participant }}
              <span class="pill" :class="node.online !== false ? 'pill--succeeded' : 'pill--idle'">
                <i></i>{{ node.online !== false ? '在线' : '离线' }}
              </span>
            </span>
          </div>
          <div v-if="node.duration" class="row"><span class="k">耗时</span><span class="v mono">{{ node.duration }}</span></div>
          <div v-if="node.failureReason" class="row">
            <span class="k">失败原因</span>
            <span class="v fail">{{ node.failureReason }}</span>
          </div>
        </div>

        <template v-if="node.participant">
          <button class="btn-ink" @click="copyTakeover">
            {{ copyState === 'ok' ? '已复制 ✓' : '复制接管提示词' }}
          </button>
          <p v-if="copyState.startsWith('err:')" class="copy-err">
            复制失败：{{ copyState.slice(4) }}
          </p>
          <p class="hint">将接管提示词复制到剪贴板，人工在任意终端粘贴即可接管该节点。</p>
        </template>
      </template>
      <p v-else class="empty">点击画布中的节点查看详情</p>
    </div>

    <div v-show="tab === 'events'" class="body">
      <div v-for="(e, i) in events" :key="i" class="evt">
        <span class="e-t">{{ new Date(e.created_at * 1000).toLocaleTimeString() }}</span>
        <i class="e-dot" :style="{ background: `var(--st-${evtGroup(e.event_type)}-main)` }"></i>
        <span class="e-m">{{ evtText(e) }}</span>
      </div>
      <p v-if="!events.length" class="empty">暂无事件</p>
    </div>
  </aside>
</template>

<style scoped>
.insp {
  display: flex; flex-direction: column; overflow: hidden;
  background: var(--surface);
  border-left: 1px solid var(--border);
}
.tabs { display: flex; border-bottom: 1px solid var(--border); }
.tabs button {
  flex: 1; height: 36px; border: 0; background: transparent;
  font-size: 12px; font-family: var(--font-ui); color: var(--text-secondary);
  cursor: pointer; border-bottom: 2px solid transparent;
  transition: all 150ms;
}
.tabs button.on { color: var(--text-primary); font-weight: 600; border-bottom-color: var(--ink); }

.body { flex: 1; overflow-y: auto; padding: 14px; }
.kv { display: flex; flex-direction: column; }
.row {
  display: flex; align-items: center; gap: 10px;
  padding: 6px 0; border-bottom: 1px dashed var(--border);
}
.row:last-child { border-bottom: 0; }
.k { flex: 0 0 64px; font-size: 11px; color: var(--text-tertiary); }
.v { font-size: 12px; flex: 1; display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.mono { font-family: var(--font-mono); font-size: 11px; }
.fail { color: var(--st-failed-deep); }
.ava { width: 18px; height: 18px; border-radius: 50%; image-rendering: pixelated; }

.pill {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: 10px; font-weight: 600; border-radius: 999px; padding: 2px 8px;
}
.pill i { width: 6px; height: 6px; border-radius: 50%; }
.pill--running   { background: var(--st-running-soft);   color: var(--st-running-deep); }   .pill--running i   { background: var(--st-running-main); }
.pill--waiting   { background: var(--st-waiting-soft);   color: var(--st-waiting-deep); }   .pill--waiting i   { background: var(--st-waiting-main); }
.pill--blocked   { background: var(--st-blocked-soft);   color: var(--st-blocked-deep); }   .pill--blocked i   { background: var(--st-blocked-main); }
.pill--succeeded { background: var(--st-succeeded-soft); color: var(--st-succeeded-deep); } .pill--succeeded i { background: var(--st-succeeded-main); }
.pill--failed    { background: var(--st-failed-soft);    color: var(--st-failed-deep); }    .pill--failed i    { background: var(--st-failed-main); }
.pill--idle      { background: var(--st-idle-soft);      color: var(--st-idle-deep); }      .pill--idle i      { background: var(--st-idle-main); }
.pill--cancelled { background: var(--st-cancelled-soft); color: var(--st-cancelled-deep); } .pill--cancelled i { background: var(--st-cancelled-main); }

.btn-ink {
  width: 100%; margin-top: 14px; height: 32px;
  border: 0; border-radius: 999px;
  background: var(--ink); color: var(--text-inverse);
  font-size: 12px; font-family: var(--font-ui); cursor: pointer;
  transition: all 150ms;
}
.btn-ink:hover { background: var(--ink-hover); }
.btn-ink:active { transform: scale(.98); }
.copy-err { margin-top: 8px; font-size: 11px; color: var(--st-failed-deep); line-height: 1.5; }
.hint { font-size: 10px; color: var(--text-tertiary); margin-top: 8px; line-height: 1.6; }
.empty { padding: 24px 4px; font-size: 11px; color: var(--text-tertiary); text-align: center; }

.evt { display: flex; gap: 8px; padding: 7px 0; border-bottom: 1px dashed var(--border); font-size: 11px; line-height: 1.5; }
.evt:last-child { border-bottom: 0; }
.e-t { font-family: var(--font-mono); font-size: 10px; color: var(--text-tertiary); flex: 0 0 52px; padding-top: 1px; }
.e-dot { width: 6px; height: 6px; border-radius: 50%; margin-top: 5px; flex: 0 0 auto; }
.e-m { color: var(--text-secondary); }
.e-m :deep(b) { color: var(--text-primary); font-weight: 600; }

.body::-webkit-scrollbar { width: 6px; }
.body::-webkit-scrollbar-thumb { background: transparent; border-radius: 3px; }
.body:hover::-webkit-scrollbar-thumb { background: var(--border-strong); }
</style>
