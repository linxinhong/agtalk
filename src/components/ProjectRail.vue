<script setup lang="ts">
/**
 * ProjectRail · 多图工程管理栏（本次新增）
 * 替代原「运行下拉选择」：工程树（工程 → GraphRun 列表），
 * 支持折叠、搜索、状态聚合点。
 *
 * props.projects: [{
 *   name, repo,
 *   runs: [{ id, status(7组), createdAt(文本), }]
 * }]（runs 按时间倒序，首个即最新）
 * emit: select({ project, run })
 */
import { computed, ref } from 'vue';

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
const props = defineProps<{ projects: RailProject[]; activeRunId?: string }>();
const emit = defineEmits(['select']);

const query = ref('');
const collapsed = ref(new Set());

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return props.projects;
  return props.projects
    .map(p => ({
      ...p,
      runs: p.runs.filter(r =>
        r.id.toLowerCase().includes(q) || p.name.toLowerCase().includes(q)),
    }))
    .filter(p => p.runs.length || p.name.toLowerCase().includes(q));
});

/** 工程聚合状态点 = 最新一次运行的状态 */
function aggStatus(p: RailProject) {
  return p.runs[0]?.status || 'idle';
}
function toggle(name: string) {
  collapsed.value.has(name) ? collapsed.value.delete(name) : collapsed.value.add(name);
  collapsed.value = new Set(collapsed.value); // 触发响应式
}
function isOpen(p: RailProject) {
  // 搜索时强制展开；默认展开含活动运行的工程
  if (query.value.trim()) return true;
  return !collapsed.value.has(p.name);
}
</script>

<template>
  <aside class="rail">
    <div class="rail-head">
      <h4>图工程</h4>
      <span class="cnt">{{ projects.length }}</span>
    </div>
    <div class="rail-search">
      <input v-model="query" type="text" placeholder="搜索工程或 run id…">
    </div>

    <div class="rail-list">
      <div v-for="p in filtered" :key="p.name" class="proj" :class="{ open: isOpen(p) }">
        <div class="proj-head" @click="toggle(p.name)">
          <span class="chev">▶</span>
          <i class="pj-dot" :style="{ background: `var(--st-${aggStatus(p)}-main)` }"></i>
          <span class="pj-name" :title="`${p.name} · ${p.repo}`">{{ p.name }}</span>
          <span class="pj-n">{{ p.runs.length }}</span>
        </div>
        <div class="proj-runs">
          <div
            v-for="r in p.runs" :key="r.id"
            class="run" :class="{ on: r.id === activeRunId }"
            @click="emit('select', { project: p, run: r })"
          >
            <i class="r-dot" :style="{ background: `var(--st-${r.status}-main)` }"></i>
            <span class="r-id">{{ r.id }}</span>
            <span class="r-time">{{ r.createdAt }}</span>
          </div>
        </div>
      </div>
      <p v-if="!filtered.length" class="rail-empty">无匹配的工程或运行</p>
    </div>
  </aside>
</template>

<style scoped>
.rail {
  display: flex; flex-direction: column; overflow: hidden;
  background: var(--surface);
  border-right: 1px solid var(--border);
}
.rail-head { padding: 12px 14px 8px; display: flex; align-items: center; justify-content: space-between; }
.rail-head h4 {
  font-size: var(--fs-meta); font-weight: 600;
  letter-spacing: var(--ls-meta); text-transform: uppercase;
  color: var(--text-tertiary);
}
.cnt {
  font-family: var(--font-mono); font-size: 10px; color: var(--text-tertiary);
  background: var(--surface-2); border-radius: 999px; padding: 1px 7px;
}
.rail-search { margin: 0 12px 10px; }
.rail-search input {
  width: 100%; height: 28px; padding: 0 10px; box-sizing: border-box;
  border: 1px solid var(--border); border-radius: var(--r-card);
  background: var(--surface-2); font-size: 12px; font-family: var(--font-ui);
  color: var(--text-primary); outline: none;
  transition: border-color 150ms, background 150ms;
}
.rail-search input:focus { border-color: var(--border-strong); background: var(--surface); }

.rail-list { flex: 1; overflow-y: auto; padding: 0 8px 12px; }
.proj { margin-bottom: 4px; }
.proj-head {
  display: flex; align-items: center; gap: 8px; padding: 7px 8px;
  border-radius: var(--r-card); cursor: pointer;
  transition: background 150ms;
}
.proj-head:hover { background: var(--surface-2); }
.chev { font-size: 9px; color: var(--text-tertiary); width: 10px; transition: transform 150ms; }
.proj.open .chev { transform: rotate(90deg); }
.pj-dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
.pj-name {
  font-size: 12px; font-weight: 600; flex: 1;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.pj-n { font-family: var(--font-mono); font-size: 10px; color: var(--text-tertiary); }

.proj-runs {
  margin: 2px 0 6px 18px; padding-left: 6px;
  border-left: 1px solid var(--border);
  display: none;
}
.proj.open .proj-runs { display: block; }
.run {
  display: flex; align-items: center; gap: 8px; padding: 6px 8px;
  border-radius: var(--r-card); cursor: pointer;
  transition: background 150ms;
}
.run:hover { background: var(--surface-2); }
.run.on { background: var(--surface-3); }
.r-dot { width: 7px; height: 7px; border-radius: 50%; flex: 0 0 auto; }
.r-id {
  font-family: var(--font-mono); font-size: 11px; flex: 1;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.r-time { font-size: 10px; color: var(--text-tertiary); }
.rail-empty { padding: 16px 14px; font-size: 11px; color: var(--text-tertiary); }

/* 细滚动条，hover 显现 */
.rail-list::-webkit-scrollbar { width: 6px; }
.rail-list::-webkit-scrollbar-thumb { background: transparent; border-radius: 3px; }
.rail-list:hover::-webkit-scrollbar-thumb { background: var(--border-strong); }
</style>
