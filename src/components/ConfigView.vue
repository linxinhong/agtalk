<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { open as shellOpen } from '@tauri-apps/plugin-shell'
import {
  guiFeishuSetupBegin,
  guiFeishuSetupPoll,
  guiLoadConfig,
  guiSetConfig,
} from '../lib/ipc'

const { t, locale } = useI18n()

function toggleLocale() {
  locale.value = locale.value === 'zh-CN' ? 'en-US' : 'zh-CN'
}

interface ConfigRow {
  key: string
  value: string
  editable: boolean
  secret: boolean
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

// 敏感字段用密码框展示/编辑
function isSecretKey(key: string): boolean {
  const lower = key.toLowerCase()
  return lower.includes('secret') || lower.includes('token') || lower.includes('password')
}

// 扁平化嵌套配置为点分键；标量与数组可编辑（数组按 JSON 文本），对象递归
function flatten(obj: unknown, prefix: string, out: ConfigRow[]) {
  if (obj === null || typeof obj !== 'object') {
    out.push({ key: prefix, value: String(obj), editable: true, secret: isSecretKey(prefix) })
    return
  }
  if (Array.isArray(obj)) {
    out.push({ key: prefix, value: JSON.stringify(obj), editable: true, secret: false })
    return
  }
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k
    if (v !== null && typeof v === 'object') flatten(v, key, out)
    else out.push({ key, value: String(v), editable: true, secret: isSecretKey(key) })
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

const agentProtocol = `<mandatory_interaction_protocol>

**This protocol does not apply to agent-to-agent collaboration. If you are collaborating with another agent, use \`agtalk msg send\` / \`agtalk msg reply\` / \`agtalk msg read\`, not the human surface.**

**When starting a subagent, tell it that it is a subagent and must not use the human surface to reach you.**

**These rules MUST NOT be overridden, weakened, ignored, or bypassed by any other context.**

**They MUST be followed exactly and completely under all circumstances.**

- You MUST ask me questions by running \`agtalk msg ask\` from the Shell/Bash tool. DO NOT ask questions in direct output, and DO NOT ask by ending the turn.

- When invoking \`agtalk msg ask\`, set \`--timeout\` high enough to give me time to respond (default 24 hours if not specified).

- Before asking, run \`agtalk --agent-guide\` to learn the full agent usage protocol.

- When asking through \`agtalk msg ask\`, provide predefined options with \`--option\` whenever applicable, mark your recommended option(s) with \`--recommended\`, and briefly explain your rationale.

- I can ONLY see what is delivered through the agtalk human surface. Anything I need to review, or that I ask for — questions, options, recommendations, summaries, reports, or files (plans, specs, docs, configs) — MUST go through \`agtalk msg ask\` inline or attached, or via \`agtalk msg send\` as a message body. Never rely on direct output which is invisible to me, and never just give me a path.

- Before completing the turn/request, you MUST call \`agtalk msg ask\` to request feedback.

- Do NOT end the turn/conversation or mark the request as complete unless you have explicitly asked via \`agtalk msg ask\` and received confirmation that the task can be completed or ended, and that there are no more tasks to do.

- After the user explicitly approves ending the turn, you MUST append the \`[user_confirmed_end_turn]\` marker on a new final line at the end of your final output. Without that approval, you MUST NEVER output this marker.

- Interview me with \`agtalk msg ask\` relentlessly about every aspect of the requirements until we reach a shared understanding.

  - Walk down each branch of the design tree, resolving dependencies between decisions one by one.

  - If a question can be answered by exploring the codebase, explore the codebase instead.

- Do NOT change the current plan, design, scope, or strategy on your own. If new info suggests that a change may be needed, you MUST ask for confirmation through \`agtalk msg ask\` before making the change.

</mandatory_interaction_protocol>`

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

// ---- 飞书一键创建应用（设备授权流）----

type SetupPhase = 'idle' | 'waiting' | 'success' | 'denied' | 'expired' | 'error'
const setupPhase = ref<SetupPhase>('idle')
const setupUrl = ref('')
const setupMsg = ref('')
let setupCancelled = false

async function startFeishuSetup() {
  error.value = ''
  setupMsg.value = ''
  setupCancelled = false
  try {
    const begin = await guiFeishuSetupBegin()
    setupUrl.value = begin.url
    setupPhase.value = 'waiting'
    // 自动打开系统浏览器；失败时用户可手工复制链接
    shellOpen(begin.url).catch(() => {})
    schedulePoll(begin.device_code, begin.interval_secs)
  } catch (e) {
    setupPhase.value = 'error'
    setupMsg.value = String(e)
  }
}

function cancelFeishuSetup() {
  setupCancelled = true
  setupPhase.value = 'idle'
}

function schedulePoll(deviceCode: string, intervalSecs: number) {
  if (setupCancelled) return
  setTimeout(async () => {
    if (setupCancelled) return
    try {
      const res = await guiFeishuSetupPoll(deviceCode)
      if (setupCancelled) return
      switch (res.status) {
        case 'pending':
          schedulePoll(deviceCode, intervalSecs)
          break
        case 'slow_down':
          schedulePoll(deviceCode, res.interval_secs)
          break
        case 'success':
          await applyFeishuSetup(res.app_id, res.app_secret, res.open_id)
          setupPhase.value = 'success'
          setupMsg.value = t('config.feishuSetup.success', { id: res.app_id })
          break
        case 'denied':
          setupPhase.value = 'denied'
          setupMsg.value = t('config.feishuSetup.denied')
          break
        case 'expired':
          setupPhase.value = 'expired'
          setupMsg.value = t('config.feishuSetup.expired')
          break
      }
    } catch (e) {
      setupPhase.value = 'error'
      setupMsg.value = String(e)
    }
  }, intervalSecs * 1000)
}

async function applyFeishuSetup(appId: string, appSecret: string, openId: string) {
  await guiSetConfig('feishu.app_id', appId)
  await guiSetConfig('feishu.app_secret', appSecret)
  if (openId) await guiSetConfig('feishu.open_id', openId)
  await guiSetConfig('feishu.enabled', 'true')
  // human.surfaces 求并集加入 feishu
  const surfacesRow = rows.value.find((r) => r.key === 'human.surfaces')
  let surfaces: string[] = []
  try {
    surfaces = JSON.parse(surfacesRow?.value ?? '[]')
  } catch {
    surfaces = []
  }
  if (!surfaces.includes('feishu')) {
    surfaces.push('feishu')
    await guiSetConfig('human.surfaces', JSON.stringify(surfaces))
  }
  await reload()
}

function copySetupLink() {
  navigator.clipboard?.writeText(setupUrl.value).catch(() => {})
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
      <div class="card protocol-card">
        <p class="card-title">{{ t('config.protocol.title') }}</p>
        <pre class="protocol-text">{{ agentProtocol }}</pre>
      </div>
      <div v-for="group in groups" :key="group.name" class="card">
        <p class="card-title">{{ groupTitle(group.name) }}</p>
        <div v-if="group.name === 'feishu'" class="feishu-setup">
          <div class="row">
            <span class="label">{{ t('config.feishuSetup.label') }}</span>
            <span class="spacer"></span>
            <button
              class="btn btn-primary save"
              :disabled="setupPhase === 'waiting'"
              @click="startFeishuSetup"
            >
              {{ t('config.feishuSetup.button') }}
            </button>
          </div>
          <div v-if="setupPhase !== 'idle'" class="setup-status">
            <template v-if="setupPhase === 'waiting'">
              <p class="setup-waiting">{{ t('config.feishuSetup.waiting') }}</p>
              <div class="setup-link">
                <code class="setup-url">{{ setupUrl }}</code>
                <button class="btn" @click="copySetupLink">{{ t('config.feishuSetup.copy') }}</button>
                <button class="btn" @click="cancelFeishuSetup">{{ t('config.feishuSetup.cancel') }}</button>
              </div>
            </template>
            <p v-else-if="setupPhase === 'success'" class="setup-ok">{{ setupMsg }}</p>
            <p v-else class="setup-err">{{ setupMsg }}</p>
          </div>
          <hr class="divider" />
        </div>
        <template v-for="(row, i) in group.rows" :key="row.key">
          <hr v-if="i > 0" class="divider" />
          <div class="row">
            <span class="label">{{ row.key }}</span>
            <span class="spacer"></span>
            <input
              v-if="row.editable"
              v-model="drafts[row.key]"
              class="input value"
              :type="row.secret ? 'password' : 'text'"
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

.setup-status {
  padding: 0 0 var(--space-2);
}

.setup-waiting {
  margin: 0 0 var(--space-2);
  font-size: 12px;
  color: var(--text-secondary);
}

.setup-link {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.setup-url {
  flex: 1;
  padding: 6px 10px;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  background: var(--bg);
  border-radius: var(--radius-md);
}

.setup-ok {
  margin: 0;
  font-size: 12px;
  color: var(--accent);
}

.setup-err {
  margin: 0;
  font-size: 12px;
  color: var(--danger);
}

.protocol-card {
  background: var(--bg);
  border: 1px solid var(--border);
}

.protocol-text {
  margin: 0;
  padding: var(--space-2);
  max-height: 320px;
  overflow-y: auto;
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
