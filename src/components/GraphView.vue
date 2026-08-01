<script setup lang="ts">
// 图工程管理界面（M4，docs/design_graph.md §4.3）：Vue Flow 画布 + 事件日志 + 操作面板。
// 数据：daemon REST（经 Tauri 命令桥，human token 在 Rust 侧）；实时：Rust 侧 SSE → Tauri event。
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { VueFlow } from '@vue-flow/core'
import dagre from '@dagrejs/dagre'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import {
  graphCancel,
  graphEvents,
  graphList,
  graphShow,
  graphStreamStart,
  graphStreamStop,
  onGraphEvent,
  type GraphEventDto,
  type GraphRunDetail,
  type GraphRunSummary,
  type GraphStreamEvent,
  type GraphEventUnlisten,
} from '../lib/graph'

// ---- 状态 ----
const runs = ref<GraphRunSummary[]>([])
const selectedRunId = ref<string>('')
const detail = ref<GraphRunDetail | null>(null)
const events = ref<GraphEventDto[]>([])
const error = ref('')
const statusFilter = ref('')
const loading = ref(false)

// 图由 Agent 生成并提交（agtalk graph run <spec.yaml>），GUI 只做加载/查看管理
// 画布（宽松类型：Vue Flow 泛型过深会触发 TS2589；运行时结构固定）
const nodes = ref<any[]>([])
const edges = ref<any[]>([])
const selectedNode = ref<GraphRunDetail['nodes'][number] | null>(null)

let unlisten: GraphEventUnlisten | null = null

const statusLabel: Record<string, string> = {
  draft: '草稿',
  validating: '校验中',
  ready: '就绪',
  running: '运行中',
  paused: '暂停',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
}

const nodeStatusLabel: Record<string, string> = {
  pending: '等待',
  ready: '就绪',
  leased: '租约',
  dispatched: '已派发',
  running: '执行中',
  verifying: '验证中',
  waiting_approval: '待审批',
  succeeded: '成功',
  failed: '失败',
  blocked: '阻塞',
  timed_out: '超时',
  cancelled: '已取消',
}

const triggerLabel: Record<string, string> = {
  on_success: '成功',
  on_failure: '失败',
  on_blocked: '阻塞',
  always: '始终',
}

// ---- 加载 ----
async function loadRuns() {
  loading.value = true
  error.value = ''
  try {
    const msg = await graphList(statusFilter.value || undefined)
    if (msg.type === 'graph_run_list') {
      runs.value = msg.runs
      if (!selectedRunId.value && msg.runs.length > 0) {
        selectedRunId.value = msg.runs[0].id
      }
    } else if (msg.type === 'error') {
      error.value = msg.message
    }
  } finally {
    loading.value = false
  }
}

async function loadDetail(runId: string) {
  detail.value = null
  selectedNode.value = null
  try {
    const msg = await graphShow(runId)
    if (msg.type === 'graph_run_detail') {
      detail.value = msg
      buildGraph(msg)
      // 初始日志
      const evMsg = await graphEvents(runId)
      if (evMsg.type === 'graph_events_result') {
        events.value = evMsg.events
      }
    } else if (msg.type === 'error') {
      error.value = msg.message
    }
  } catch (e) {
    error.value = String(e)
  }
}

// ---- 画布构建 ----
function buildGraph(d: GraphRunDetail) {
  const flowNodes: any[] = d.nodes.map((n) => ({
    id: n.node_key,
    type: 'default',
    position: { x: 0, y: 0 },
    data: { label: `${n.node_key}\n[${nodeStatusLabel[n.status] ?? n.status}]` },
    class: `agtalk-node agtalk-node-${n.status}`,
  }))
  const flowEdges: any[] = d.edges.map((e, i) => ({
    id: `e-${i}`,
    source: e.from,
    target: e.to,
    label: triggerLabel[e.trigger] ?? e.trigger,
    class: `agtalk-edge agtalk-edge-${e.trigger}`,
  }))
  const laid = layout(flowNodes, flowEdges)
  nodes.value = laid.nodes
  edges.value = laid.edges
}

function layout(
  ns: any[],
  es: any[],
): { nodes: any[]; edges: Array<Record<string, unknown>> } {
  const g = new dagre.graphlib.Graph()
  g.setDefaultEdgeLabel(() => ({}))
  g.setGraph({ rankdir: 'LR', nodesep: 50, ranksep: 80, marginx: 20, marginy: 20 })
  ns.forEach((n) => g.setNode(n.id, { width: 200, height: 56 }))
  es.forEach((e) => g.setEdge(e.source, e.target))
  dagre.layout(g)
  const laid = ns.map((n) => {
    const p = g.node(n.id)
    return { ...n, position: { x: p.x - 100, y: p.y - 28 } }
  })
  return { nodes: laid, edges: es }
}

function onNodeClick(e: { node: { id: string } }) {
  const key = e.node.id
  selectedNode.value =
    detail.value?.nodes.find((n) => n.node_key === key) ?? null
}

// ---- 实时事件 ----
function handleStreamEvent(payload: GraphStreamEvent) {
  if (payload.run_id !== selectedRunId.value) return
  let evt: GraphEventDto
  try {
    evt = JSON.parse(payload.data)
  } catch {
    return
  }
  appendEvent(evt)
  applyEvent(evt)
}

function appendEvent(evt: GraphEventDto) {
  if (events.value.some((e) => e.id === evt.id)) return
  events.value.push(evt)
  if (events.value.length > 500) events.value.shift()
}

function applyEvent(evt: GraphEventDto) {
  // 节点状态变化 → 更新画布
  if (evt.event_type.startsWith('node_') && evt.node_key) {
    const status = evt.event_type.slice('node_'.length)
    const n = nodes.value.find((x) => x.id === evt.node_key)
    if (n) {
      n.class = `agtalk-node agtalk-node-${status}`
      ;(n.data as { label: string }).label = `${evt.node_key}\n[${nodeStatusLabel[status] ?? status}]`
    }
    const d = detail.value?.nodes.find((x) => x.node_key === evt.node_key)
    if (d) d.status = status
  } else if (evt.event_type.startsWith('graph_')) {
    if (detail.value) detail.value.run.status = evt.event_type.slice('graph_'.length)
  }
}

// ---- 操作 ----
async function doCancel() {
  if (!selectedRunId.value) return
  try {
    const msg = await graphCancel(selectedRunId.value)
    if (msg.type === 'error') error.value = msg.message
    else await loadDetail(selectedRunId.value)
  } catch (e) {
    error.value = String(e)
  }
}

// ---- 生命周期 ----
watch(selectedRunId, (id) => {
  if (!id) return
  if (unlisten) {
    // 切换 run：旧订阅停止由 Rust 侧标记处理；这里重启新订阅
  }
  void graphStreamStart(id)
  void loadDetail(id)
})

onMounted(async () => {
  unlisten = await onGraphEvent(handleStreamEvent)
  await loadRuns()
  if (selectedRunId.value) {
    void graphStreamStart(selectedRunId.value)
    void loadDetail(selectedRunId.value)
  }
})

onUnmounted(() => {
  unlisten?.()
  if (selectedRunId.value) void graphStreamStop(selectedRunId.value)
})
</script>

<template>
  <div class="graph-view">
    <header class="gv-header">
      <h1>图工程</h1>
      <select v-model="statusFilter" class="gv-select" @change="loadRuns">
        <option value="">全部状态</option>
        <option value="running">运行中</option>
        <option value="paused">暂停</option>
        <option value="completed">已完成</option>
        <option value="failed">失败</option>
      </select>
      <select v-model="selectedRunId" class="gv-select gv-run-select">
        <option v-for="r in runs" :key="r.id" :value="r.id">
          {{ r.id.slice(0, 8) }} · {{ statusLabel[r.status] ?? r.status }} · {{ r.goal }}
        </option>
      </select>
      <button class="gv-btn" @click="loadRuns">刷新</button>
      <button class="gv-btn gv-btn-danger" :disabled="!selectedRunId" @click="doCancel">
        取消运行
      </button>
      <span class="gv-hint">图由 Agent 生成并提交（agtalk graph run &lt;spec.yaml&gt;），本界面只做加载与查看</span>
    </header>

    <div v-if="error" class="gv-error">{{ error }}</div>

    <div v-if="!selectedRunId && !loading" class="gv-empty">
      暂无 GraphRun。用 agtalk graph run &lt;spec.yaml&gt; 提交一个图开始。
    </div>

    <main v-else class="gv-main">
      <section class="gv-canvas">
        <VueFlow
          :nodes="nodes"
          :edges="edges"
          :fit-view-on-init="true"
          :min-zoom="0.2"
          :max-zoom="2"
          @node-click="onNodeClick"
        />
        <div v-if="detail" class="gv-run-meta">
          {{ detail.run.id }} · {{ statusLabel[detail.run.status] ?? detail.run.status }} ·
          {{ detail.run.repository ?? '' }}
        </div>
      </section>

      <aside class="gv-side">
        <section class="gv-panel">
          <h3>节点详情</h3>
          <div v-if="selectedNode" class="gv-node-detail">
            <div class="gv-kv">
              <span>节点</span><b>{{ selectedNode.node_key }}</b>
            </div>
            <div class="gv-kv">
              <span>类型</span><b>{{ selectedNode.node_type }}</b>
            </div>
            <div class="gv-kv">
              <span>状态</span
              ><b :class="['gv-status', 'gv-status-' + selectedNode.status]">{{
                nodeStatusLabel[selectedNode.status] ?? selectedNode.status
              }}</b>
            </div>
            <div class="gv-kv">
              <span>尝试</span><b>attempt {{ selectedNode.attempt }}</b>
            </div>
            <div class="gv-kv">
              <span>执行者</span><b>{{ selectedNode.participant_id ?? '-' }}</b>
            </div>
            <div v-if="selectedNode.started_at && selectedNode.completed_at" class="gv-kv">
              <span>耗时</span><b>{{ ((selectedNode.completed_at - selectedNode.started_at)).toFixed(1) }}s</b>
            </div>
            <div v-if="selectedNode.failure_detail" class="gv-fail">
              {{ selectedNode.failure_type }}: {{ selectedNode.failure_detail }}
            </div>
          </div>
          <div v-else class="gv-muted">点击画布节点查看详情</div>
        </section>

        <section class="gv-panel gv-log">
          <h3>事件日志</h3>
          <div class="gv-log-list">
            <div v-for="e in [...events].reverse()" :key="e.id" class="gv-log-item">
              <span class="gv-log-id">#{{ e.id }}</span>
              <span class="gv-log-type">{{ e.event_type }}</span>
              <span class="gv-log-node">{{ e.node_key ?? '' }}</span>
            </div>
            <div v-if="events.length === 0" class="gv-muted">暂无事件</div>
          </div>
        </section>
      </aside>
    </main>
  </div>
</template>

<style scoped>
.graph-view {
  display: flex;
  flex-direction: column;
  height: 100vh;
  font-family: var(--font-ui, system-ui, sans-serif);
  color: var(--text-primary, #1d1d1f);
  background: var(--bg, #ffffff);
}
.gv-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border, #e5e7eb);
  flex-wrap: wrap;
}
.gv-header h1 {
  font-size: 16px;
  margin: 0 12px 0 0;
}
.gv-select {
  padding: 4px 8px;
  border: 1px solid var(--border, #d1d5db);
  border-radius: 6px;
  background: var(--bg, #fff);
  color: var(--text-primary, #1d1d1f);
  max-width: 320px;
}
.gv-run-select {
  flex: 1;
  min-width: 200px;
}
.gv-btn {
  padding: 5px 12px;
  border: 1px solid var(--border, #d1d5db);
  border-radius: 6px;
  background: var(--bg, #fff);
  color: var(--text-primary, #1d1d1f);
  cursor: pointer;
}
.gv-btn-primary {
  background: var(--accent, #2563eb);
  border-color: var(--accent, #2563eb);
  color: #fff;
}
.gv-btn-danger {
  border-color: #ef4444;
  color: #ef4444;
}
.gv-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.gv-error {
  margin: 8px 14px;
  padding: 8px 12px;
  background: #fef2f2;
  color: #b91c1c;
  border-radius: 6px;
  font-size: 13px;
}
.gv-submit {
  margin: 10px 14px;
  padding: 10px;
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.gv-submit textarea {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 12px;
  border: 1px solid var(--border, #d1d5db);
  border-radius: 6px;
  padding: 8px;
  background: var(--bg, #fff);
  color: var(--text-primary, #1d1d1f);
}
.gv-submit-actions {
  display: flex;
  gap: 8px;
}
.gv-empty {
  padding: 40px;
  text-align: center;
  color: var(--text-secondary, rgba(0,0,0,0.55));
}
.gv-main {
  flex: 1;
  display: flex;
  min-height: 0;
}
.gv-canvas {
  flex: 1;
  position: relative;
  min-width: 0;
}
.gv-run-meta {
  position: absolute;
  top: 8px;
  left: 8px;
  z-index: 5;
  font-size: 12px;
  background: var(--bg-elevated, rgba(0, 0, 0, 0.03));
  backdrop-filter: blur(4px);
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid var(--border, #e5e7eb);
}
.gv-side {
  width: 320px;
  border-left: 1px solid var(--border, #e5e7eb);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.gv-panel {
  padding: 10px 12px;
  border-bottom: 1px solid var(--border, #e5e7eb);
}
.gv-panel h3 {
  margin: 0 0 8px;
  font-size: 13px;
  color: var(--text-secondary, rgba(0,0,0,0.55));
}
.gv-kv {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 13px;
  padding: 2px 0;
}
.gv-kv span {
  color: var(--text-secondary, rgba(0,0,0,0.55));
}
.gv-fail {
  margin-top: 6px;
  font-size: 12px;
  color: #b91c1c;
  background: #fef2f2;
  padding: 6px 8px;
  border-radius: 6px;
}
.gv-status {
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 12px;
}
.gv-log {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.gv-log-list {
  flex: 1;
  overflow-y: auto;
  font-size: 12px;
  font-family: var(--font-mono, ui-monospace, monospace);
}
.gv-log-item {
  display: flex;
  gap: 8px;
  padding: 2px 0;
  border-bottom: 1px dashed var(--border, #f3f4f6);
}
.gv-log-id {
  color: var(--text-tertiary, rgba(0,0,0,0.38));
}
.gv-log-type {
  color: var(--accent, #2563eb);
}
.gv-log-node {
  color: var(--text-secondary, rgba(0,0,0,0.55));
}
.gv-muted {
  color: var(--text-tertiary, rgba(0,0,0,0.38));
  font-size: 12px;
}
</style>

<style>
/* 节点状态着色（Vue Flow 节点 class 由库注入，需非 scoped） */
.agtalk-node-pending {
  --vf-node-bg: #f3f4f6;
  --vf-node-border: #9ca3af;
}
.agtalk-node-ready,
.agtalk-node-leased,
.agtalk-node-dispatched {
  --vf-node-bg: #eff6ff;
  --vf-node-border: #3b82f6;
}
.agtalk-node-running {
  --vf-node-bg: #dbeafe;
  --vf-node-border: #2563eb;
}
.agtalk-node-verifying {
  --vf-node-bg: #e0e7ff;
  --vf-node-border: #6366f1;
}
.agtalk-node-succeeded {
  --vf-node-bg: #dcfce7;
  --vf-node-border: #16a34a;
}
.agtalk-node-failed {
  --vf-node-bg: #fee2e2;
  --vf-node-border: #dc2626;
}
.agtalk-node-waiting_approval {
  --vf-node-bg: #fef9c3;
  --vf-node-border: #ca8a04;
}
.agtalk-node-blocked {
  --vf-node-bg: #ffedd5;
  --vf-node-border: #ea580c;
}
.agtalk-node-timed_out,
.agtalk-node-cancelled {
  --vf-node-bg: #f3f4f6;
  --vf-node-border: #6b7280;
}
.agtalk-edge-on_failure {
  stroke: #dc2626;
  stroke-dasharray: 5 3;
}
.agtalk-edge-on_blocked {
  stroke: #ea580c;
  stroke-dasharray: 2 3;
}
.agtalk-edge-always {
  stroke: #9ca3af;
}
</style>
