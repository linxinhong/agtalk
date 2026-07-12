<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { closePopup, popupCancel, popupDone, popupLoad, popupReply, type PopupView } from '../lib/ipc'

const { t } = useI18n()

const view = ref<PopupView | null>(null)
const error = ref('')
const busy = ref(false)
const resolved = ref(false)
const replyBody = ref('')
const selected = ref<string[]>([])

interface ApprovalMeta {
  choices: string[]
  recommended?: string
  single: boolean
  select_only: boolean
}

const approval = computed<ApprovalMeta | null>(() => {
  const msg = view.value?.message
  if (!msg || msg.content_type !== 'approval_request') return null
  try {
    const meta = JSON.parse(msg.metadata)
    return {
      choices: Array.isArray(meta.choices) ? meta.choices : [],
      recommended: typeof meta.recommended === 'string' ? meta.recommended : undefined,
      single: Boolean(meta.single),
      select_only: Boolean(meta.select_only),
    }
  } catch {
    return { choices: [], single: false, select_only: false }
  }
})

/** 相对时间：<5s 刚刚，<60s N 秒前，<1h N 分钟前，<24h N 小时前，否则绝对日期。 */
const relTime = computed(() => {
  const created = view.value?.message.created_at
  if (!created) return ''
  const secs = Math.max(0, Math.floor(Date.now() / 1000 - created))
  if (secs < 5) return t('popup.time.justNow')
  if (secs < 60) return t('popup.time.secondsAgo', { n: secs })
  if (secs < 3600) return t('popup.time.minutesAgo', { n: Math.floor(secs / 60) })
  if (secs < 86400) return t('popup.time.hoursAgo', { n: Math.floor(secs / 3600) })
  return new Date(created * 1000).toLocaleDateString()
})

onMounted(async () => {
  try {
    view.value = await popupLoad()
  } catch (e) {
    error.value = String(e)
  }
})

function handleError(e: unknown) {
  const msg = String(e)
  if (msg.includes('already_resolved')) {
    resolved.value = true
    return
  }
  error.value = msg
}

/** 勾选/取消勾选选项；single 审批表现为单选。 */
function toggleChoice(c: string) {
  const a = approval.value
  if (!a || busy.value || resolved.value) return
  if (a.single) {
    selected.value = selected.value.includes(c) ? [] : [c]
  } else {
    selected.value = selected.value.includes(c)
      ? selected.value.filter((x) => x !== c)
      : [...selected.value, c]
  }
}

/** 提交可用性：审批需勾选或（非 select_only 时）有补充文本；文本消息需回复正文。 */
const canSubmit = computed(() => {
  if (busy.value || resolved.value) return false
  if (approval.value) {
    if (selected.value.length > 0) return true
    return !approval.value.select_only && replyBody.value.trim() !== ''
  }
  return replyBody.value.trim() !== ''
})

async function submitReply() {
  if (!canSubmit.value) return
  busy.value = true
  error.value = ''
  try {
    await popupReply(replyBody.value, selected.value)
    await closePopup()
  } catch (e) {
    handleError(e)
  } finally {
    busy.value = false
  }
}

async function submitDone() {
  if (busy.value || resolved.value) return
  busy.value = true
  error.value = ''
  try {
    await popupDone()
    await closePopup()
  } catch (e) {
    handleError(e)
  } finally {
    busy.value = false
  }
}

async function submitCancel() {
  if (busy.value || resolved.value) return
  busy.value = true
  error.value = ''
  try {
    await popupCancel()
    await closePopup()
  } catch (e) {
    handleError(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="popup">
    <div v-if="error && !view" class="popup-status">
      <p class="status-error">{{ t('popup.loadFailed') }}: {{ error }}</p>
    </div>
    <div v-else-if="!view" class="popup-status">
      <p class="status-loading">{{ t('popup.loading') }}</p>
    </div>

    <template v-else>
      <header class="navbar" data-tauri-drag-region>
        <span class="brand">
          <span class="brand-dot"></span>
          <span class="brand-title">{{ t('popup.fromTitle', { name: view.message.from_name }) }}</span>
          <span v-if="view.message.subject" class="brand-chip">
            <span class="chip-text">{{ view.message.subject }}</span>
          </span>
          <span v-if="relTime" class="brand-time">· {{ relTime }}</span>
        </span>
      </header>

      <main class="content">
        <pre class="plain-body">{{ view.message.body }}</pre>

        <div v-if="resolved" class="resolved">{{ t('popup.resolved') }}</div>

        <template v-else>
          <div v-if="approval && approval.choices.length" class="options">
            <div
              v-for="c in approval.choices"
              :key="c"
              class="option"
              :class="{ selected: selected.includes(c), single: approval.single }"
              role="checkbox"
              :aria-checked="selected.includes(c)"
              tabindex="0"
              @click="toggleChoice(c)"
              @keydown.enter.prevent="toggleChoice(c)"
              @keydown.space.prevent="toggleChoice(c)"
            >
              <span class="check" :class="{ radio: approval.single }">{{
                approval.single ? '' : selected.includes(c) ? '✓' : ''
              }}</span>
              <span class="label">
                <span v-if="c === approval.recommended" class="rec-badge">
                  <span class="rec-badge-pill">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3z" /><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
                    {{ t('popup.recommended') }}
                  </span>
                </span>
                {{ c }}
              </span>
            </div>
          </div>

          <textarea
            v-if="!approval || !approval.select_only"
            v-model="replyBody"
            class="textarea"
            :placeholder="approval ? t('popup.supplementPlaceholder') : t('popup.replyPlaceholder')"
            :disabled="busy"
            rows="2"
          />

          <p v-if="error" class="error">{{ error }}</p>
        </template>
      </main>

      <footer v-if="!resolved" class="footer">
        <button class="btn cancel" type="button" :disabled="busy" @click="submitCancel">
          {{ t('popup.cancel') }}
        </button>
        <span class="spacer"></span>
        <button v-if="!approval" class="btn" type="button" :disabled="busy" @click="submitDone">
          {{ t('popup.done') }}
        </button>
        <button class="btn btn-primary" type="button" :disabled="!canSubmit" @click="submitReply">
          {{ approval ? t('popup.submit') : t('popup.reply') }}
        </button>
      </footer>
    </template>
  </div>
</template>

<style scoped>
/* 视觉对齐 AskHuman 弹窗：macOS 风 token（颜色/圆角/间距），深浅色两套 */
.popup {
  --accent: #0a84ff;
  --accent-green: #34c759;
  --rec-badge-fg: #1c9b41;
  --radius-sm: 6px;
  --radius-md: 8px;
  --bg: #ffffff;
  --bg-elevated: rgba(0, 0, 0, 0.03);
  --card-bg: rgba(0, 0, 0, 0.035);
  --border: rgba(0, 0, 0, 0.14);
  --text-primary: #1d1d1f;
  --text-secondary: rgba(0, 0, 0, 0.55);
  --text-tertiary: rgba(0, 0, 0, 0.38);
  --control-bg: #ffffff;
  --control-border: rgba(0, 0, 0, 0.2);
  --danger: #d43b3b;
  display: flex;
  flex-direction: column;
  height: 100vh;
  box-sizing: border-box;
  background: var(--bg);
  color: var(--text-primary);
  font: 13px/1.45 -apple-system, 'SF Pro Text', system-ui, 'PingFang SC', sans-serif;
}

@media (prefers-color-scheme: dark) {
  .popup {
    --accent-green: #30d158;
    --rec-badge-fg: #30d158;
    --bg: #1e1e1e;
    --bg-elevated: rgba(255, 255, 255, 0.06);
    --card-bg: rgba(255, 255, 255, 0.06);
    --border: rgba(255, 255, 255, 0.16);
    --text-primary: #e8e8ea;
    --text-secondary: rgba(255, 255, 255, 0.62);
    --text-tertiary: rgba(255, 255, 255, 0.4);
    --control-bg: rgba(255, 255, 255, 0.08);
    --control-border: rgba(255, 255, 255, 0.22);
    --danger: #e55b5b;
  }
}

/* ===== 状态页（加载中/加载失败） ===== */
.popup-status {
  margin: auto;
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
}

.status-error {
  color: var(--danger);
}

/* ===== 头部 ===== */
.navbar {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border);
}

.brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.brand-dot {
  position: relative;
  flex: 0 0 auto;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--accent-green);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-green) 22%, transparent);
  animation: brand-dot-breathe 2.4s ease-in-out infinite;
}

@keyframes brand-dot-breathe {
  0%,
  100% {
    opacity: 0.85;
    transform: scale(1);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}

.brand-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brand-chip {
  flex: 0 1 auto;
  display: inline-flex;
  align-items: center;
  max-width: 140px;
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
}

.chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.brand-time {
  flex: 0 100000 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0.7;
}

/* ===== 内容区 ===== */
.content {
  flex: 1 1 auto;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 10px 12px 6px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.plain-body {
  flex: 1 1 auto;
  min-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 14px;
  line-height: 1.6;
  color: var(--text-primary);
  margin: 0;
}

.resolved {
  color: var(--accent);
  font-size: 13px;
}

/* ===== 审批选项（卡片式勾选，对齐 AskHuman .option） ===== */
.options {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.option {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 7px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  background: var(--card-bg);
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease;
}

.option:hover {
  background: var(--bg-elevated);
}

.option.selected {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.option .check {
  flex: 0 0 auto;
  width: 18px;
  height: 18px;
  margin-top: 1px;
  border-radius: 5px;
  border: 1.5px solid var(--control-border);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 11px;
  line-height: 1;
}

.option.selected .check {
  background: var(--accent);
  border-color: var(--accent);
}

/* 单选：方形勾选框渲染为圆形 radio，选中时中心实心圆点 */
.option .check.radio {
  border-radius: 50%;
}

.option.selected .check.radio {
  background: transparent;
  border-color: var(--accent);
  position: relative;
}

.option.selected .check.radio::after {
  content: '';
  position: absolute;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--accent);
}

.option .label {
  flex: 1 1 auto;
  font-size: 14px;
  line-height: 20px;
  color: var(--text-primary);
  word-break: break-word;
}

/* 推荐 Badge：绿色胶囊（大拇指 + 「推荐」），与飞书卡片的绿色[推荐]对齐 */
.rec-badge {
  display: inline-flex;
  align-items: center;
  vertical-align: top;
  height: 20px;
  margin-right: 6px;
}

.rec-badge-pill {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  height: 18px;
  padding: 0 7px 0 5px;
  border-radius: var(--radius-sm);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
  color: var(--rec-badge-fg);
  background: color-mix(in srgb, var(--accent-green) 22%, transparent);
}

.rec-badge-pill svg {
  width: 11px;
  height: 11px;
}

/* ===== 回复输入 ===== */
.textarea {
  font-family: inherit;
  font-size: 13px;
  line-height: 1.5;
  width: 100%;
  min-height: 40px;
  resize: none;
  padding: 6px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--control-border);
  background: var(--control-bg);
  color: var(--text-primary);
  box-sizing: border-box;
}

.textarea::placeholder {
  color: var(--text-tertiary);
}

.error {
  margin: 0;
  font-size: 12px;
  color: var(--danger);
}

/* ===== 底部按钮区 ===== */
.footer {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-top: 1px solid var(--border);
}

.footer .spacer {
  flex: 1 1 auto;
}

.btn {
  font-family: inherit;
  font-size: 13px;
  font-weight: 500;
  line-height: 1;
  padding: 7px 16px;
  border-radius: var(--radius-md);
  border: 1px solid var(--control-border);
  background: var(--control-bg);
  color: var(--text-primary);
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
  user-select: none;
}

.btn:hover {
  background: var(--bg-elevated);
}

.btn:disabled {
  opacity: 0.4;
  pointer-events: none;
}

.btn-primary {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}

.btn-primary:hover {
  background: color-mix(in srgb, var(--accent) 88%, black);
}

.btn.cancel {
  border-color: color-mix(in srgb, var(--danger) 50%, transparent);
  color: var(--danger);
}

.btn.cancel:hover {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}
</style>
