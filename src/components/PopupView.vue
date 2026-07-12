<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { closePopup, popupDone, popupLoad, popupReply, type PopupView } from '../lib/ipc'

const { t } = useI18n()

const view = ref<PopupView | null>(null)
const error = ref('')
const busy = ref(false)
const resolved = ref(false)
const replyBody = ref('')

interface ApprovalMeta {
  choices: string[]
  recommended?: string
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
      select_only: Boolean(meta.select_only),
    }
  } catch {
    return { choices: [], select_only: false }
  }
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

async function submitReply(choice?: string) {
  if (busy.value || resolved.value) return
  busy.value = true
  error.value = ''
  try {
    await popupReply(replyBody.value, choice)
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
</script>

<template>
  <div class="popup">
    <div v-if="error && !view" class="state">{{ t('popup.loadFailed') }}: {{ error }}</div>
    <div v-else-if="!view" class="state">{{ t('popup.loading') }}</div>
    <template v-else>
      <header class="head">
        <span class="from">{{ view.message.from_name }}</span>
        <span v-if="view.message.subject" class="subject">{{ view.message.subject }}</span>
      </header>

      <main class="body">{{ view.message.body }}</main>

      <div v-if="resolved" class="state resolved">{{ t('popup.resolved') }}</div>

      <template v-else>
        <div v-if="approval" class="choices">
          <button
            v-for="c in approval.choices"
            :key="c"
            class="choice"
            :class="{ recommended: c === approval.recommended }"
            :disabled="busy"
            @click="submitReply(c)"
          >
            {{ c }}
            <span v-if="c === approval.recommended" class="tag">{{ t('popup.recommended') }}</span>
          </button>
        </div>

        <textarea
          v-if="!approval || !approval.select_only"
          v-model="replyBody"
          class="reply"
          :placeholder="t('popup.replyPlaceholder')"
          :disabled="busy"
          rows="2"
        />

        <div v-if="error" class="error">{{ error }}</div>

        <footer class="actions">
          <button class="later" :disabled="busy" @click="closePopup">{{ t('popup.later') }}</button>
          <button v-if="!approval" class="done" :disabled="busy" @click="submitDone">
            {{ t('popup.done') }}
          </button>
          <button
            v-if="!approval || !approval.select_only"
            class="reply-btn"
            :disabled="busy || !replyBody.trim()"
            @click="submitReply()"
          >
            {{ t('popup.reply') }}
          </button>
        </footer>
      </template>
    </template>
  </div>
</template>

<style scoped>
.popup {
  --bg: #ffffff;
  --text: #1c1c1e;
  --muted: #6e6e73;
  --accent: #0a6cff;
  --border: #d8d8dc;
  --choice-bg: #f2f2f5;
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 10px 12px;
  box-sizing: border-box;
  background: var(--bg);
  color: var(--text);
  font: 13px/1.45 -apple-system, 'PingFang SC', sans-serif;
}

@media (prefers-color-scheme: dark) {
  .popup {
    --bg: #1e1e20;
    --text: #f2f2f5;
    --muted: #9a9aa0;
    --accent: #4a8cff;
    --border: #3a3a40;
    --choice-bg: #2c2c30;
  }
}

.state {
  margin: auto;
  color: var(--muted);
  text-align: center;
}

.resolved {
  color: var(--accent);
}

.head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 6px;
}

.from {
  font-weight: 600;
}

.subject {
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.body {
  flex: 1;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  margin-bottom: 8px;
}

.choices {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 8px;
}

.choice {
  padding: 4px 12px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--choice-bg);
  color: var(--text);
  cursor: pointer;
}

.choice.recommended {
  border-color: var(--accent);
}

.tag {
  margin-left: 4px;
  font-size: 10px;
  color: var(--accent);
}

.reply {
  resize: none;
  margin-bottom: 8px;
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--choice-bg);
  color: var(--text);
  font: inherit;
}

.error {
  margin-bottom: 6px;
  font-size: 12px;
  color: #d43b3b;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.actions button {
  padding: 5px 14px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--choice-bg);
  color: var(--text);
  cursor: pointer;
}

.actions .reply-btn,
.actions .done {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.actions button:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
