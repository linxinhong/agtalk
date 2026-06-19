<script setup lang="ts">
import { ref, onMounted, computed, watch } from "vue";
import InboxView from "./views/InboxView.vue";
import ConversationView from "./views/ConversationView.vue";
import SettingsView from "./views/SettingsView.vue";
import { listParticipants, pingDaemon } from "./lib/ipc";
import type { Participant } from "./lib/types";

const STORAGE_KEY = "agtalk.gui.participant";

type Tab = "inbox" | "conversations" | "settings";

const daemonOnline = ref(false);
const daemonChecking = ref(true);
const currentTab = ref<Tab>("inbox");
const participants = ref<Participant[]>([]);
const participantsLoading = ref(false);
const currentParticipant = ref<string>("");
const error = ref<string | null>(null);

async function checkDaemon() {
  daemonChecking.value = true;
  try {
    daemonOnline.value = await pingDaemon();
    if (daemonOnline.value) {
      await loadParticipants();
    }
  } catch {
    daemonOnline.value = false;
  } finally {
    daemonChecking.value = false;
  }
}

async function loadParticipants() {
  participantsLoading.value = true;
  error.value = null;
  try {
    participants.value = await listParticipants();
    resolveCurrentParticipant();
  } catch (e) {
    console.error("加载参与者失败:", e);
    error.value = "加载参与者失败";
  } finally {
    participantsLoading.value = false;
  }
}

function resolveCurrentParticipant() {
  if (participants.value.length === 0) {
    currentParticipant.value = "";
    return;
  }
  const saved = localStorage.getItem(STORAGE_KEY) || "";
  if (saved && participants.value.some((p) => p.name === saved)) {
    currentParticipant.value = saved;
    return;
  }
  const me = participants.value.find((p) => p.name === "me");
  if (me) {
    currentParticipant.value = me.name;
    return;
  }
  const human = participants.value.find((p) => p.type === "human");
  if (human) {
    currentParticipant.value = human.name;
    return;
  }
  currentParticipant.value = participants.value[0].name;
}

function onParticipantChange(name: string) {
  currentParticipant.value = name;
  localStorage.setItem(STORAGE_KEY, name);
}

const currentParticipantDisplay = computed(() => {
  const p = participants.value.find((p) => p.name === currentParticipant.value);
  if (!p) return currentParticipant.value;
  return `${p.name} (${p.type}${p.role ? ` / ${p.role}` : ""})`;
});

onMounted(() => {
  checkDaemon();
});

watch(daemonOnline, (online) => {
  if (online && participants.value.length === 0) {
    loadParticipants();
  }
});
</script>

<template>
  <div class="app-shell">
    <header class="topbar">
      <div class="brand">agtalk</div>

      <nav class="tabs">
        <button
          class="tab"
          :class="{ active: currentTab === 'inbox' }"
          @click="currentTab = 'inbox'"
        >
          Inbox
        </button>
        <button
          class="tab"
          :class="{ active: currentTab === 'conversations' }"
          @click="currentTab = 'conversations'"
        >
          Conversations
        </button>
        <button
          class="tab"
          :class="{ active: currentTab === 'settings' }"
          @click="currentTab = 'settings'"
        >
          Settings
        </button>
      </nav>

      <div class="topbar-right">
        <select
          v-if="participants.length > 0"
          class="participant-select"
          :value="currentParticipant"
          @change="onParticipantChange(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="p in participants" :key="p.name" :value="p.name">
            {{ p.name }} ({{ p.type }}{{ p.role ? ` / ${p.role}` : "" }})
          </option>
        </select>

        <div class="daemon-status" :class="{ online: daemonOnline, offline: !daemonOnline && !daemonChecking }">
          <span class="status-dot"></span>
          <span v-if="daemonChecking">检测中</span>
          <span v-else-if="daemonOnline">在线</span>
          <span v-else>离线</span>
          <button v-if="!daemonOnline && !daemonChecking" class="retry-btn" @click="checkDaemon">重试</button>
        </div>
      </div>
    </header>

    <main class="main-area">
      <div v-if="!daemonOnline && !daemonChecking" class="daemon-banner">
        daemon 未运行，请运行 <code>agtalk daemon start</code>
        <button class="retry-btn" @click="checkDaemon">重试</button>
      </div>

      <div v-if="daemonOnline && participants.length === 0 && !participantsLoading" class="empty-state">
        没有可用参与者，请先使用 <code>agtalk join</code> 加入网络。
      </div>

      <InboxView
        v-if="currentTab === 'inbox' && daemonOnline && currentParticipant"
        :participant="currentParticipant"
      />
      <ConversationView
        v-else-if="currentTab === 'conversations' && daemonOnline && currentParticipant"
        :daemon-online="daemonOnline"
        :current-participant="currentParticipant"
      />
      <SettingsView
        v-else-if="currentTab === 'settings'"
        :participant="currentParticipant"
        :daemon-online="daemonOnline"
      />
    </main>
  </div>
</template>

<style scoped>
.topbar {
  height: 48px;
  flex: 0 0 48px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-sidebar);
  gap: 16px;
}

.brand {
  font-weight: 700;
  font-size: 15px;
  color: var(--text);
  white-space: nowrap;
}

.tabs {
  display: flex;
  gap: 4px;
  background: var(--bg);
  padding: 3px;
  border-radius: var(--radius);
  border: 1px solid var(--border);
}

.tab {
  padding: 5px 14px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 13px;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s;
}

.tab:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.tab.active {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 500;
}

.topbar-right {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

.participant-select {
  min-width: 160px;
  padding: 5px 10px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.daemon-status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary);
}

.daemon-status.online { color: var(--accent-green, #34c759); }
.daemon-status.offline { color: var(--danger); }

.status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
}

.retry-btn {
  padding: 2px 8px;
  border: 1px solid currentColor;
  background: transparent;
  color: currentColor;
  border-radius: 4px;
  cursor: pointer;
  font-size: 11px;
}

.daemon-banner {
  background: #fff3cd;
  border-bottom: 1px solid #ffc107;
  padding: 8px 16px;
  font-size: 13px;
  color: #664d03;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  flex-shrink: 0;
}

.daemon-banner code {
  background: rgba(0, 0, 0, 0.06);
  padding: 2px 6px;
  border-radius: 3px;
  font-size: 12px;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 14px;
  gap: 8px;
}

.empty-state code {
  background: var(--bg-hover);
  padding: 2px 6px;
  border-radius: 4px;
}
</style>
