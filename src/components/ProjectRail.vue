<script setup lang="ts">
/**
 * ProjectRail · 左侧运行列表（扁平）：直接显示 run id + 状态点 + 相对时间。
 * 支持搜索（run id / goal）、选中高亮。
 *
 * props.runs: [{ id, status(7组), createdAt(文本), repository }]（时间倒序）
 * emit: select({ run })
 */
import { computed, ref } from 'vue'

interface RailRun {
  id: string
  status: string
  createdAt: string
  repository: string | null
}
const props = defineProps<{ runs: RailRun[]; activeRunId?: string }>();
const emit = defineEmits(['select', 'delete']);

const query = ref('');

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return props.runs;
  return props.runs.filter((r) => r.id.toLowerCase().includes(q));
});
</script>

<template>
  <aside class="rail">
    <div class="rail-head">
      <h4>图运行</h4>
      <span class="cnt">{{ runs.length }}</span>
    </div>
    <div class="rail-search">
      <input v-model="query" type="text" placeholder="搜索 run id…">
    </div>

    <div class="rail-list">
      <div
        v-for="r in filtered"
        :key="r.id"
        class="run"
        :class="{ on: r.id === activeRunId }"
        @click="emit('select', { run: r })"
      >
        <i class="r-dot" :style="{ background: `var(--st-${r.status}-main)` }"></i>
        <span class="r-id" :title="r.repository ?? ''">{{ r.id }}</span>
        <span class="r-time">{{ r.createdAt }}</span>
        <button
          class="run-del"
          title="删除（仅已结束的图；级联清理）"
          @click.stop="emit('delete', r.id)"
        >×</button>
      </div>
      <p v-if="!filtered.length" class="rail-empty">无匹配的运行</p>
    </div>
  </aside>
</template>

<style scoped>
.rail {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--surface);
  border-right: 1px solid var(--border);
}
.rail-head {
  padding: 12px 14px 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.rail-head h4 {
  font-size: var(--fs-meta);
  font-weight: 600;
  letter-spacing: var(--ls-meta);
  text-transform: uppercase;
  color: var(--text-tertiary);
  margin: 0;
}
.cnt {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-tertiary);
  background: var(--surface-2);
  border-radius: 999px;
  padding: 1px 7px;
}
.rail-search {
  margin: 0 12px 10px;
}
.rail-search input {
  width: 100%;
  height: 28px;
  padding: 0 10px;
  box-sizing: border-box;
  border: 1px solid var(--border);
  border-radius: var(--r-card);
  background: var(--surface-2);
  font-size: 12px;
  font-family: var(--font-ui);
  color: var(--text-primary);
  outline: none;
  transition: border-color 150ms, background 150ms;
}
.rail-search input:focus {
  border-color: var(--border-strong);
  background: var(--surface);
}
.rail-list {
  flex: 1;
  overflow-y: auto;
  padding: 0 8px 12px;
}
.run {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  border-radius: var(--r-card);
  cursor: pointer;
  transition: background 150ms;
}
.run:hover {
  background: var(--surface-2);
}
.run.on {
  background: var(--surface-3);
}
.r-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex: 0 0 auto;
}
.r-id {
  font-family: var(--font-mono);
  font-size: 11px;
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-primary);
}
.r-time {
  font-size: 10px;
  color: var(--text-tertiary);
  flex-shrink: 0;
}
.run-del {
  flex: 0 0 auto;
  width: 18px;
  height: 18px;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--text-tertiary);
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  transition: all 150ms;
}
.run:hover .run-del {
  opacity: 1;
}
.run-del:hover {
  background: var(--st-failed-tint);
  color: var(--st-failed-deep);
}
.rail-empty {
  padding: 16px 14px;
  font-size: 11px;
  color: var(--text-tertiary);
}

.rail-list::-webkit-scrollbar {
  width: 6px;
}
.rail-list::-webkit-scrollbar-thumb {
  background: transparent;
  border-radius: 3px;
}
.rail-list:hover::-webkit-scrollbar-thumb {
  background: var(--border-strong);
}
</style>
