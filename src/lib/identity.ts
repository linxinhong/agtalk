// 执行者身份工具：djb2 确定性头像映射 + 状态归组（设计系统 v2，DESIGN.md §2 Q2）
// 同名同像；头像资源 public/avatars/avatar_NNN.png

export const AVATAR_COUNT = 180

export function djb2(s: string): number {
  let h = 5381
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0
  return h
}

/** participant → 确定性头像 URL；空 participant 返回 null（渲染占位虚线圆）。
 *  资源集：public/avatars/avatar_001.png … avatar_180.png（用户自定义 180 张，已重命名）。 */
export function avatarFor(participant?: string | null): string | null {
  if (!participant) return null
  const n = (djb2(participant) % AVATAR_COUNT) + 1 // 1..180
  return `/avatars/avatar_${String(n).padStart(3, '0')}.png` // 绝对路径（tauri custom-protocol 根）
}

/** 12 个生命周期状态 → 7 个视觉组（与 tokens.css --st-* 对应） */
export const STATUS_GROUP: Record<string, string> = {
  pending: 'idle',
  ready: 'idle',
  leased: 'running',
  dispatched: 'running',
  running: 'running',
  verifying: 'running',
  waiting_approval: 'waiting',
  blocked: 'blocked',
  succeeded: 'succeeded',
  failed: 'failed',
  timed_out: 'failed',
  cancelled: 'cancelled',
}

export const STATUS_TEXT: Record<string, string> = {
  idle: '未开始',
  running: '执行中',
  waiting: '待审批',
  blocked: '阻塞',
  succeeded: '成功',
  failed: '失败',
  cancelled: '已取消',
}

export const STRUCT_TYPES = ['join', 'gate', 'approval']

export function statusGroupOf(rawStatus?: string | null): string {
  return (rawStatus && STATUS_GROUP[rawStatus]) || 'idle'
}
