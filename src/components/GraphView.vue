<script setup lang="ts">
// 图工程 GUI · 三区主视图（设计系统 v2：工程栏 / 画布 / 检查器 + 顶栏）
// 数据：Tauri 命令桥（graph.ts，human token 在 Rust 侧）；实时：Rust 侧 SSE → Tauri event。
// 布局：dagre rankdir=LR，节点 216×64（SegmentedNode 的 --gn-w 可收窄同步）。

import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { VueFlow, useVueFlow, MarkerType } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls } from '@vue-flow/controls'
import dagre from '@dagrejs/dagre'

import ProjectRail from '../components/ProjectRail.vue'
import GraphHeader from '../components/GraphHeader.vue'
import NodeInspector from '../components/NodeInspector.vue'
import SegmentedNode from '../components/SegmentedNode.vue'
import { statusGroupOf } from '../lib/identity'
import {
  graphControl,
  graphEvents,
  graphList,
  graphShow,
  graphStreamStart,
  graphStreamStop,
  onGraphEvent,
  type GraphEventDto,
  type GraphNodeDetail,
  type GraphRunSummary,
  type GraphStreamEvent,
  type GraphEventUnlisten,
} from '../lib/graph'

const nodeTypes = { agtalk: SegmentedNode as any }
const { fitView } = useVueFlow()

// ---- 状态 ----
interface RailRun {
  id: string
  status: string
  createdAt: string
  repository: string | null
}
interface RailProject {
  name: string
  repo: string
  runs: RailRun[]
}
const projects = ref<RailProject[]>([])
const active = ref({ project: '', runId: '', repo: '', status: 'idle' })
const filter = ref('all')
const nodes = ref<any[]>([])
const edges = ref<any[]>([])
const events = ref<GraphEventDto[]>([])
const selectedNode = ref<any>(null)
const loading = ref(false)
const loadError = ref('') // 显式错误条（历史教训：不许静默"暂无"）
let unlisten: GraphEventUnlisten | null = null
let durTimer: ReturnType<typeof setInterval> | null = null

// ---- 工程树：扁平 runs 按 repository 分组（后端无 /projects，前端分组） ----
function fmtTime(ts: number): string {
  const d = new Date(ts * 1000)
  const diff = Date.now() / 1000 - ts
  if (diff < 60) return `${Math.floor(diff)}s`
  if (diff < 3600) return `${Math.floor(diff / 60)}m`
  return d.toLocaleDateString()
}
function buildProjects(runs: GraphRunSummary[]): RailProject[] {
  const byRepo = new Map<string, RailProject>()
  for (const r of runs) {
    const repo = r.repository || '本地'
    const name = repo.split('/').pop() || repo
    if (!byRepo.has(repo)) byRepo.set(repo, { name, repo, runs: [] })
    byRepo.get(repo)!.runs.push({
      id: r.id,
      status: statusGroupOf(r.status),
      createdAt: fmtTime(r.created_at),
      repository: repo,
    })
  }
  const list = [...byRepo.values()]
  // 每工程 runs 按时间倒序（首个即最新）
  for (const p of list) p.runs.sort((a, b) => b.id.localeCompare(a.id))
  return list
}

// ---- 加载 ----
async function loadProjects() {
  loading.value = true
  loadError.value = ''
  try {
    const msg = await graphList(undefined)
    if (msg.type === 'graph_run_list') {
      projects.value = buildProjects(msg.runs)
      // 默认选中最新运行
      const first = projects.value[0]?.runs[0]
      if (first && !active.value.runId) {
        await selectRun({ project: projects.value[0], run: first })
      }
    } else if (msg.type === 'error') {
      loadError.value = msg.message
    }
  } catch (e) {
    loadError.value = String(e)
  } finally {
    loading.value = false
  }
}

// ---- 节点 data 映射（GraphNodeDetail → SegmentedNode data） ----
function fmtDuration(startedAt: number | null, completedAt: number | null): string {
  const s = startedAt ?? 0
  const e = completedAt ?? Date.now() / 1000
  if (!s) return ''
  const sec = Math.max(0, Math.round(e - s))
  const m = Math.floor(sec / 60)
  const ss = String(sec % 60).padStart(2, '0')
  return `${m}:${ss}`
}
function nodeData(n: GraphNodeDetail, runId: string) {
  return {
    runId,
    nodeKey: n.node_key,
    nodeType: n.node_type,
    status: n.status,
    group: statusGroupOf(n.status),
    participant: n.participant_id || null,
    online: n.participant_online,
    attempt: n.attempt || 1,
    duration: fmtDuration(n.started_at, n.completed_at),
    failureReason: n.failure_detail || '',
  }
}

// ---- dagre 布局（rankdir=LR，216×64） ----
const NODE_W = 216
const NODE_H = 64
function layoutGraph(detail: { nodes: GraphNodeDetail[]; edges: { from: string; to: string; trigger: string }[] }) {
  const g = new dagre.graphlib.Graph()
  g.setGraph({ rankdir: 'LR', nodesep: 60, ranksep: 80, marginx: 20, marginy: 20 })
  g.setDefaultEdgeLabel(() => ({}))
  detail.nodes.forEach((n) => g.setNode(n.node_key, { width: NODE_W, height: NODE_H }))
  detail.edges.forEach((e) => g.setEdge(e.from, e.to))
  dagre.layout(g)

  const runId = active.value.runId
  nodes.value = detail.nodes.map((n) => {
    const { x, y } = g.node(n.node_key)
    return {
      id: n.node_key,
      type: 'agtalk',
      position: { x: x - NODE_W / 2, y: y - NODE_H / 2 },
      data: nodeData(n, runId),
    }
  })
  edges.value = detail.edges.map((e, i) => ({
    id: `e-${i}`,
    source: e.from,
    target: e.to,
    label: e.trigger === 'on_success' ? '' : e.trigger === 'on_failure' ? 'failure' : e.trigger,
    class: e.trigger === 'on_failure' ? 'edge-fail' : '',
    style:
      e.trigger === 'on_failure'
        ? { stroke: 'var(--st-failed-main)', strokeDasharray: '5 4' }
        : { stroke: 'var(--border-strong)' },
    markerEnd: {
      type: MarkerType.ArrowClosed,
      color: e.trigger === 'on_failure' ? 'var(--st-failed-main)' : 'var(--border-strong)',
    },
  }))
}

async function selectRun({ project, run }: { project: RailProject; run: RailRun }) {
  active.value = { project: project.name, runId: run.id, repo: project.repo, status: run.status }
  selectedNode.value = null
  loadError.value = ''
  try {
    const msg = await graphShow(run.id)
    if (msg.type === 'graph_run_detail') {
      layoutGraph(msg)
      await startStream(run.id)
      await loadEvents(run.id)
      fitView({ padding: 0.15 })
    } else if (msg.type === 'error') {
      loadError.value = msg.message
    }
  } catch (e) {
    loadError.value = String(e)
  }
}

async function loadEvents(runId: string) {
  try {
    const msg = await graphEvents(runId, undefined)
    if (msg.type === 'graph_events_result') events.value = msg.events
  } catch {
    /* 事件加载失败不阻塞画布 */
  }
}

// ---- SSE：只改 data，不重建节点 ----
async function startStream(runId: string) {
  await graphStreamStop(runId)
  unlisten?.()
  unlisten = await onGraphEvent((e: GraphStreamEvent) => {
    if (e.run_id !== runId) return
    let evt: GraphEventDto
    try {
      evt = JSON.parse(e.data)
    } catch {
      return
    }
    events.value = [evt, ...events.value]
    if (evt.event_type.startsWith('node_') && evt.node_key) {
      const status = evt.event_type.slice('node_'.length)
      const n = nodes.value.find((x) => x.id === evt.node_key)
      if (n) {
        n.data = { ...n.data, status, group: statusGroupOf(status) }
        if (selectedNode.value?.nodeKey === evt.node_key) selectedNode.value = { ...n.data }
      }
    } else if (evt.event_type.startsWith('graph_')) {
      active.value.status = statusGroupOf(evt.event_type.slice('graph_'.length))
    }
  })
  await graphStreamStart(runId)
}

// ---- 交互 ----
function onNodeClick({ node }: { node: { id: string } }) {
  const n = nodes.value.find((x) => x.id === node.id)
  if (n) selectedNode.value = { ...n.data }
}

const filteredNodes = computed(() => {
  if (filter.value === 'all') return nodes.value
  if (filter.value === 'done')
    return nodes.value.map((n) => ({
      ...n,
      hidden: !['succeeded', 'failed', 'cancelled'].includes(n.data.group),
    }))
  return nodes.value.map((n) => ({ ...n, hidden: n.data.group !== filter.value }))
})

async function doControl(action: string) {
  if (!active.value.runId) return
  try {
    const msg = await graphControl(active.value.runId, action)
    if (msg.type === 'error') loadError.value = msg.message
    else await selectRun({ project: { name: active.value.project, repo: active.value.repo, runs: [] }, run: { id: active.value.runId, status: 'idle', createdAt: '', repository: active.value.repo } })
  } catch (e) {
    loadError.value = String(e)
  }
}

// running 节点耗时实时刷新（1s）
function startDurTimer() {
  durTimer && clearInterval(durTimer)
  durTimer = setInterval(() => {
    if (!nodes.value.length) return
    nodes.value.forEach((n) => {
      if (n.data.group === 'running' && n.data.status !== 'running' && n.data.status !== 'verifying') return
      // 仅对进行中的节点刷新耗时（started_at 未知时跳过）
    })
  }, 1000)
}

onMounted(async () => {
  await loadProjects()
  startDurTimer()
})
onBeforeUnmount(() => {
  unlisten?.()
  if (active.value.runId) graphStreamStop(active.value.runId)
  if (durTimer) clearInterval(durTimer)
})

// 侧栏 node 需带 runId（NodeInspector 复制提示词用）
const inspectorNode = computed(() =>
  selectedNode.value ? { ...selectedNode.value } : null,
)
</script>

<template>
  <div class="gv">
    <GraphHeader
      :project="active.project"
      :run-id="active.runId"
      :repo="active.repo"
      :run-status="active.status"
      :filter="filter"
      :loading="loading"
      @filter="filter = $event"
      @refresh="loadProjects"
      @pause="doControl('pause')"
      @resume="doControl('resume')"
      @cancel="doControl('cancel')"
    />

    <div v-if="loadError" class="err-bar">⚠ {{ loadError }}</div>

    <div class="gv-main">
      <ProjectRail :projects="projects" :active-run-id="active.runId" @select="selectRun" />

      <div class="gv-canvas">
        <VueFlow
          :nodes="filteredNodes"
          :edges="edges"
          :node-types="nodeTypes"
          :min-zoom="0.2"
          :max-zoom="2"
          fit-view-on-init
          @node-click="onNodeClick"
        >
          <Background :gap="20" :size="1" pattern-color="var(--canvas-dot)" />
          <Controls position="bottom-left" />
          <div v-if="active.runId" class="cv-meta">
            {{ active.runId }} · {{ nodes.length }} 节点
          </div>
          <button class="cv-fit" title="恢复默认视角（适应全部节点）" @click="fitView({ padding: 0.15 })">
            ⌂ 适应视图
          </button>
        </VueFlow>
      </div>

      <NodeInspector :node="inspectorNode" :events="events" />
    </div>
  </div>
</template>

<style scoped>
.gv {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: var(--bg);
  font-family: var(--font-ui);
}
.err-bar {
  padding: 8px 16px;
  font-size: 12px;
  background: var(--st-failed-tint);
  color: var(--st-failed-deep);
  border-bottom: 1px solid var(--st-failed-soft);
}
.gv-main {
  flex: 1;
  display: grid;
  overflow: hidden;
  grid-template-columns: 248px 1fr 300px;
}
.gv-canvas {
  position: relative;
  overflow: hidden;
  background: var(--bg);
}
.cv-fit {
  position: absolute;
  right: 12px;
  top: 10px;
  z-index: 5;
  height: 26px;
  padding: 0 12px;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  background: var(--surface);
  color: var(--text-primary);
  font-size: 11px;
  font-family: var(--font-ui);
  cursor: pointer;
  transition: all 150ms;
}
.cv-fit:hover {
  border-color: var(--text-tertiary);
  background: var(--surface-2);
}
.cv-meta {
  position: absolute;
  left: 12px;
  top: 10px;
  z-index: 5;
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-tertiary);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--r-card, 6px);
  padding: 4px 8px;
}
@media (max-width: 1100px) {
  .gv-main {
    grid-template-columns: 220px 1fr;
  }
  .gv-main > :last-child {
    display: none;
  }
}
@media (max-width: 760px) {
  .gv-main {
    grid-template-columns: 1fr;
  }
  .gv-main > :first-child {
    display: none;
  }
  .cv-meta {
    display: none;
  }
}
</style>
