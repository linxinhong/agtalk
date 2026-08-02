<script setup lang="ts">
/**
 * GraphHeader · 顶栏：面包屑（工程/运行/仓库）+ 状态筛选 + 运行操作
 * 主操作用墨色，危险操作用失败色描边——彩色只属于状态。
 *
 * props: runStatus(7组), filter, loading
 * emit: filter(status), refresh(), pause(), resume(), cancel()
 */
interface HeaderProps {
  runStatus?: string
  filter?: string
  loading?: boolean
}
defineProps<HeaderProps>();

const emit = defineEmits(['filter', 'refresh', 'pause', 'resume', 'cancel']);

const FILTERS: { key: string; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'running', label: '运行中' },
  { key: 'waiting', label: '待审批' },
  { key: 'failed', label: '失败' },
  { key: 'done', label: '已完成' },
];
</script>

<template>
  <header class="top">
    <span class="logo"><i></i>agtalk 图工程</span>

    <nav class="filters">
      <span
        v-for="f in FILTERS" :key="f.key"
        :class="{ on: filter === f.key }"
        @click="emit('filter', f.key!)"
      >{{ f.label }}</span>
    </nav>

    <div class="actions">
      <span class="hint">图由 Agent 生成并提交，本界面只做加载与查看</span>
      <button class="btn" :disabled="loading" @click="emit('refresh')">刷新</button>
      <button v-if="runStatus === 'running'" class="btn" @click="emit('pause')">暂停</button>
      <button v-if="runStatus === 'idle'" class="btn" @click="emit('resume')">恢复</button>
      <button
        v-if="['running', 'waiting', 'idle'].includes(runStatus ?? '')"
        class="btn btn--danger"
        @click="emit('cancel')"
      >取消运行</button>
    </div>
  </header>
</template>

<style scoped>
.top {
  display: flex; align-items: center; gap: 12px;
  height: 48px; padding: 0 16px; box-sizing: border-box;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  flex-wrap: nowrap; /* 窄宽度不换行，靠截断 */
}
.logo { font-size: 13px; font-weight: 700; display: flex; align-items: center; gap: 8px; }
.logo i { width: 18px; height: 18px; border-radius: 5px; background: var(--ink); }

.filters {
  display: flex; gap: 4px; margin-left: 8px;
  flex-shrink: 1; min-width: 0;
  overflow-x: auto; white-space: nowrap;
  scrollbar-width: none;
}
.filters span {
  font-size: 11px; padding: 4px 10px; border-radius: 999px;
  color: var(--text-secondary); cursor: pointer;
  transition: all 150ms;
}
.filters span:hover { background: var(--surface-2); }
.filters span.on { background: var(--ink); color: var(--text-inverse); }

.actions { margin-left: auto; display: flex; gap: 8px; align-items: center; flex-shrink: 0; }
.hint { font-size: 11px; color: var(--text-tertiary); margin-right: 4px; }

.btn {
  height: 28px; padding: 0 12px; border-radius: 999px;
  font-size: 12px; font-family: var(--font-ui); cursor: pointer;
  border: 1px solid var(--border-strong);
  background: var(--surface); color: var(--text-primary);
  transition: all 150ms;
}
.btn:hover:not(:disabled) { border-color: var(--text-tertiary); }
.btn:active:not(:disabled) { transform: scale(.98); }
.btn:disabled { opacity: .5; cursor: default; }
.btn--danger { color: var(--st-failed-deep); border-color: var(--st-failed-soft); }
.btn--danger:hover:not(:disabled) { background: var(--st-failed-tint); border-color: var(--st-failed-main); }

/* 窄屏自适应 */
@media (max-width: 900px) {
  .hint { display: none; }
  .top { gap: 8px; padding: 0 10px; }
}
@media (max-width: 700px) {
  .logo { font-size: 12px; }
  .filters { display: none; }
}
</style>
