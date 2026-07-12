<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { guiLoadConfig, guiSetConfig } from '../lib/ipc'

const { t } = useI18n()

interface ConfigRow {
  key: string
  value: string
  editable: boolean
}

const path = ref('')
const rows = ref<ConfigRow[]>([])
const drafts = reactive<Record<string, string>>({})
const error = ref('')
const savedKey = ref('')
const busyKey = ref('')

// 扁平化嵌套配置为点分键；标量可编辑，数组/对象/null 只读展示
function flatten(obj: unknown, prefix: string, out: ConfigRow[]) {
  if (obj === null || typeof obj !== 'object') {
    out.push({ key: prefix, value: String(obj), editable: true })
    return
  }
  if (Array.isArray(obj)) {
    out.push({ key: prefix, value: JSON.stringify(obj), editable: false })
    return
  }
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k
    if (v !== null && typeof v === 'object') flatten(v, key, out)
    else out.push({ key, value: String(v), editable: true })
  }
}

async function reload() {
  error.value = ''
  try {
    const view = await guiLoadConfig()
    path.value = view.path
    const flat: ConfigRow[] = []
    flatten(view.config, '', flat)
    rows.value = flat
    for (const r of flat) drafts[r.key] = r.value
  } catch (e) {
    error.value = String(e)
  }
}

onMounted(reload)

async function save(row: ConfigRow) {
  if (busyKey.value || drafts[row.key] === row.value) return
  busyKey.value = row.key
  error.value = ''
  savedKey.value = ''
  try {
    await guiSetConfig(row.key, drafts[row.key])
    await reload()
    savedKey.value = row.key
  } catch (e) {
    error.value = String(e)
  } finally {
    busyKey.value = ''
  }
}
</script>

<template>
  <div class="config">
    <header class="head">
      <h1>{{ t('config.title') }}</h1>
      <button class="reload" @click="reload">{{ t('config.reload') }}</button>
    </header>

    <div v-if="path" class="path">{{ path }}</div>

    <div v-if="error" class="error">
      {{ error }}
      <div v-if="error.includes('daemon_unreachable')" class="hint">{{ t('config.daemonHint') }}</div>
    </div>

    <main class="rows">
      <div v-for="row in rows" :key="row.key" class="row">
        <label class="key" :for="`cfg-${row.key}`">{{ row.key }}</label>
        <input
          v-if="row.editable"
          :id="`cfg-${row.key}`"
          v-model="drafts[row.key]"
          class="value"
          :disabled="busyKey === row.key"
          @keyup.enter="save(row)"
        />
        <code v-else class="value readonly">{{ row.value }}</code>
        <button
          v-if="row.editable"
          class="save"
          :disabled="busyKey === row.key || drafts[row.key] === row.value"
          @click="save(row)"
        >
          {{ savedKey === row.key ? t('config.saved') : t('config.save') }}
        </button>
      </div>
      <div v-if="!rows.length && !error" class="state">{{ t('config.loading') }}</div>
    </main>
  </div>
</template>

<style scoped>
.config {
  --bg: #ffffff;
  --text: #1c1c1e;
  --muted: #6e6e73;
  --accent: #0a6cff;
  --border: #d8d8dc;
  --field-bg: #f2f2f5;
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 16px 20px;
  box-sizing: border-box;
  background: var(--bg);
  color: var(--text);
  font: 13px/1.45 -apple-system, 'PingFang SC', sans-serif;
}

@media (prefers-color-scheme: dark) {
  .config {
    --bg: #1e1e20;
    --text: #f2f2f5;
    --muted: #9a9aa0;
    --accent: #4a8cff;
    --border: #3a3a40;
    --field-bg: #2c2c30;
  }
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.head h1 {
  margin: 0;
  font-size: 16px;
}

.reload {
  padding: 4px 12px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--field-bg);
  color: var(--text);
  cursor: pointer;
}

.path {
  margin-bottom: 12px;
  font-size: 11px;
  color: var(--muted);
  word-break: break-all;
}

.error {
  margin-bottom: 12px;
  padding: 8px 10px;
  border: 1px solid #d43b3b;
  border-radius: 6px;
  font-size: 12px;
  color: #d43b3b;
}

.hint {
  margin-top: 4px;
  color: var(--muted);
}

.rows {
  flex: 1;
  overflow-y: auto;
}

.row {
  display: grid;
  grid-template-columns: 220px 1fr auto;
  align-items: center;
  gap: 10px;
  padding: 6px 0;
  border-bottom: 1px solid var(--border);
}

.key {
  color: var(--muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.value {
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--field-bg);
  color: var(--text);
  font: inherit;
}

.readonly {
  border-color: transparent;
  color: var(--muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}

.save {
  padding: 5px 14px;
  border: 1px solid var(--accent);
  border-radius: 6px;
  background: var(--accent);
  color: #fff;
  cursor: pointer;
}

.save:disabled {
  opacity: 0.4;
  cursor: default;
}

.state {
  margin-top: 40px;
  color: var(--muted);
  text-align: center;
}
</style>
