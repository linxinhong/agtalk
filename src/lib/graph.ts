// 图工程管理界面：daemon API 封装（经 Tauri 命令桥，human token 在 Rust 侧）。
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export interface GraphRunSummary {
  id: string
  goal: string
  status: string
  repository: string | null
  base_revision: string | null
  created_at: number
  started_at: number | null
  completed_at: number | null
  failure_reason: string | null
}

export interface GraphNodeDetail {
  node_key: string
  node_type: string
  status: string
  attempt: number
  participant_id: string | null
  /** participant 是否有活跃 mailbox（认领状态：false 且未派发 → 灰） */
  participant_online: boolean
  workspace_id: string | null
  started_at: number | null
  completed_at: number | null
  failure_type: string | null
  failure_detail: string | null
}

export interface GraphEdgeDto {
  from: string
  to: string
  trigger: string
}

export interface GraphRunDetail {
  run: GraphRunSummary
  nodes: GraphNodeDetail[]
  edges: GraphEdgeDto[]
  required_approvals: string[]
  resource_conflicts: string[]
}

export interface GraphEventDto {
  id: number
  event_type: string
  node_key: string | null
  payload: Record<string, unknown>
  created_at: number
}

// ServerMsg 的 graph 变体（serde tag=type）
export interface GraphRunCreated {
  type: 'graph_run_created'
  run_id: string
  status: string
  errors: { code: string; message: string; node?: string }[]
  warnings: { code: string; message: string; node?: string }[]
}
export interface GraphRunList {
  type: 'graph_run_list'
  runs: GraphRunSummary[]
}
export interface GraphRunDetailMsg {
  type: 'graph_run_detail'
  run: GraphRunSummary
  nodes: GraphNodeDetail[]
  edges: GraphEdgeDto[]
  required_approvals: string[]
  resource_conflicts: string[]
}
export interface GraphEventsResult {
  type: 'graph_events_result'
  events: GraphEventDto[]
}
export interface GraphRunControlOk {
  type: 'graph_run_control_ok'
  run_id: string
  action: string
  status: string
}
export interface GraphNodeReportOk {
  type: 'graph_node_report_ok'
  run_id: string
  node_key: string
  attempt: number
  status: string
  message: string
}
export interface GraphError {
  type: 'error'
  code: string
  message: string
}

export type GraphServerMsg =
  | GraphRunCreated
  | GraphRunList
  | GraphRunDetailMsg
  | GraphEventsResult
  | GraphRunControlOk
  | GraphNodeReportOk
  | GraphError

export const graphList = (status?: string) =>
  invoke<GraphServerMsg>('gui_graph_list', status ? { status } : {})

export const graphShow = (runId: string) =>
  invoke<GraphServerMsg>('gui_graph_show', { runId })

export const graphEvents = (runId: string, since?: number) =>
  invoke<GraphServerMsg>('gui_graph_events', { runId, since })

export const graphSubmit = (spec: string) =>
  invoke<GraphServerMsg>('gui_graph_submit', { spec })

export const graphCancel = (runId: string) =>
  invoke<GraphServerMsg>('gui_graph_cancel', { runId })

/** 节点接管提示词（Tim 设计稿方案二：GUI 复制按钮） */
export const nodePrompt = (runId: string, nodeKey: string) =>
  invoke<string>('gui_node_prompt', { runId, nodeKey })

export const graphStreamStart = (runId: string) =>
  invoke<void>('gui_graph_stream_start', { runId })

export const graphStreamStop = (runId: string) =>
  invoke<void>('gui_graph_stream_stop', { runId })

/** Rust 侧 SSE 订阅 → Tauri event。payload: { run_id, id, data } */
export interface GraphStreamEvent {
  run_id: string
  id: string
  data: string
}

/** 订阅 GraphEvent 实时推送（返回取消函数）。 */
export const onGraphEvent = (cb: (e: GraphStreamEvent) => void) =>
  listen<GraphStreamEvent>('graph-event', (event) => cb(event.payload))

export type GraphEventUnlisten = UnlistenFn
