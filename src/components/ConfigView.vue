<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { guiLoadConfig, guiSetConfig } from '../lib/ipc'

const { t, locale } = useI18n()

function toggleLocale() {
  locale.value = locale.value === 'zh-CN' ? 'en-US' : 'zh-CN'
}

interface ConfigRow {
  key: string
  value: string
  editable: boolean
}

interface ConfigGroup {
  name: string
  rows: ConfigRow[]
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

// 按点分键首段分组为卡片；无点分的顶层键归入 general
const groups = computed<ConfigGroup[]>(() => {
  const map = new Map<string, ConfigRow[]>()
  for (const r of rows.value) {
    const name = r.key.includes('.') ? r.key.split('.')[0] : 'general'
    const list = map.get(name) ?? []
    list.push(r)
    map.set(name, list)
  }
  return [...map.entries()].map(([name, groupRows]) => ({ name, rows: groupRows }))
})

function groupTitle(name: string): string {
  const key = `config.groups.${name}`
  const translated = t(key)
  return translated === key ? name : translated
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
  <div class="settings">
    <header class="head">
      <h1>{{ t('config.title') }}</h1>
      <div class="head-actions">
        <button class="btn" @click="toggleLocale">{{ t('app.switchLanguage') }}</button>
        <button class="btn" @click="reload">{{ t('config.reload') }}</button>
      </div>
    </header>

    <div v-if="path" class="path">{{ path }}</div>

    <div v-if="error" class="error-banner">
      {{ error }}
      <div v-if="error.includes('daemon_unreachable')" class="hint">{{ t('config.daemonHint') }}</div>
    </div>

    <main class="settings-body">
      <div v-for="group in groups" :key="group.name" class="card">
        <p class="card-title">{{ groupTitle(group.name) }}</p>
        <template v-for="(row, i) in group.rows" :key="row.key">
          <hr v-if="i > 0" class="divider" />
          <div class="row">
            <span class="label">{{ row.key }}</span>
            <span class="spacer"></span>
            <input
              v-if="row.editable"
              v-model="drafts[row.key]"
              class="input value"
              :disabled="busyKey === row.key"
              @keyup.enter="save(row)"
            />
            <code v-else class="value readonly">{{ row.value }}</code>
            <button
              v-if="row.editable"
              class="btn btn-primary save"
              :disabled="busyKey === row.key || drafts[row.key] === row.value"
              @click="save(row)"
            >
              {{ savedKey === row.key ? t('config.saved') : t('config.save') }}
            </button>
          </div>
        </template>
      </div>
      <div v-if="!groups.length && !error" class="state">{{ t('config.loading') }}</div>
    </main>
  </div>
</template>

<style scoped>
.settings {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: var(--space-4) var(--space-5);
  background: var(--bg);
  color: var(--text-primary);
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-2);
}

.head h1 {
  margin: 0;
  font-size: 17px;
  font-weight: 600;
}

.head-actions {
  display: flex;
  gap: var(--space-2);
}

.path {
  margin-bottom: var(--space-3);
  font-size: 11px;
  font-family: var(--font-mono);
  color: var(--text-tertiary);
  word-break: break-all;
}

.error-banner {
  margin-bottom: var(--space-3);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--danger);
  border-radius: var(--radius-md);
  font-size: 12px;
  color: var(--danger);
}

.hint {
  margin-top: var(--space-1);
  color: var(--text-secondary);
}

.settings-body {
  flex: 1;
  overflow-y: auto;
}

.card {
  margin-bottom: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-lg);
  background: var(--card-bg);
}

.card-title {
  margin: 0 0 var(--space-2);
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) 0;
}

.label {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.spacer {
  flex: 1;
}

.value {
  width: 220px;
}

.readonly {
  padding: 6px 10px;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.save {
  min-width: 64px;
}

.divider {
  margin: 0;
  border: none;
  border-top: 1px solid var(--border);
}

.state {
  margin-top: 40px;
  color: var(--text-secondary);
  text-align: center;
}
</style>
