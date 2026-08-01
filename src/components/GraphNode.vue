<script setup lang="ts">
// Vue Flow 自定义节点（Tim 设计稿已批准）：左 36px 执行者头像 + 右两行文字。
// 必须含 Handle（target=Left / source=Right），否则自定义节点会丢失边连接点。
// struct 节点（participantId 空）头像槽渲染类型字形（gate=◇ / join=▬ / approval=⬡），
// 不在菱形/六边形节点内放头像（避开 clip-path 冲突）。
import { computed } from 'vue'
import { Handle, Position } from '@vue-flow/core'
import ParticipantAvatar from './ParticipantAvatar.vue'

const props = defineProps<{
  data: {
    nodeKey: string
    statusText: string
    claimText: string
    nodeType: string
    participantId?: string | null
    participantOnline?: boolean
  }
}>()

const GLYPHS: Record<string, string> = {
  gate: '◇',
  join: '▬',
  approval: '⬡',
}
const typeGlyph = computed(() => GLYPHS[props.data.nodeType] ?? '')
</script>

<template>
  <div class="gn-root">
    <Handle type="target" :position="Position.Left" />
    <div class="gn-body">
      <div class="gn-avatar-slot">
        <ParticipantAvatar
          v-if="props.data.participantId"
          :participant="props.data.participantId"
          :online="!!props.data.participantOnline"
        />
        <span v-else class="gn-type-glyph">{{ typeGlyph }}</span>
      </div>
      <div class="gn-text">
        <div class="gn-title">{{ props.data.nodeKey }}</div>
        <div class="gn-status">[{{ props.data.statusText }} · {{ props.data.claimText }}]</div>
      </div>
    </div>
    <Handle type="source" :position="Position.Right" />
  </div>
</template>

<style scoped>
.gn-root {
  position: relative;
  min-width: 0;
}
.gn-body {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  background: transparent; /* 节点底色由 Vue Flow wrapper 的 agtalk-node-* CSS 控制 */
}
.gn-avatar-slot {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.gn-type-glyph {
  font-size: 22px;
  line-height: 1;
  color: #64748b;
}
.gn-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.gn-title {
  font-weight: 700;
  font-size: 13px;
  color: var(--text-primary, #1f2937);
  white-space: nowrap;
}
.gn-status {
  font-size: 11px;
  color: var(--text-secondary, #6b7280);
  white-space: nowrap;
}
</style>
