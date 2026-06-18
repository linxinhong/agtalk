<script setup lang="ts">
import { ref, onMounted } from "vue";
import ConversationView from "./views/ConversationView.vue";
import SettingsView from "./views/SettingsView.vue";
import { pingDaemon } from "./lib/ipc";

const daemonOnline = ref(false);
const daemonChecking = ref(true);
const currentView = ref<"conversation" | "settings">("conversation");

async function checkDaemon() {
  daemonChecking.value = true;
  daemonOnline.value = await pingDaemon();
  daemonChecking.value = false;
}

onMounted(() => { checkDaemon(); });
</script>

<template>
  <div class="app-layout">
    <aside class="sidebar">
      <div class="sidebar-header">
        <span>agtalk</span>
        <button
          class="settings-btn"
          @click="currentView = currentView === 'conversation' ? 'settings' : 'conversation'"
        >
          {{ currentView === 'conversation' ? '⚙' : '←' }}
        </button>
      </div>

      <div v-if="daemonChecking" class="empty-state">连接中...</div>
      <div v-else-if="!daemonOnline && currentView === 'conversation'" class="empty-state sidebar-warn">
        daemon 未运行<br />
        <small>运行 agtalk daemon start</small>
      </div>
    </aside>

    <main class="main-area">
      <div v-if="!daemonOnline && !daemonChecking" class="daemon-banner">
        daemon 未运行，请运行 <code>agtalk daemon start</code>
        <button class="retry-btn" @click="checkDaemon">重试</button>
      </div>
      <ConversationView v-if="currentView === 'conversation'" :daemon-online="daemonOnline" />
      <SettingsView v-if="currentView === 'settings'" />
      <div v-if="daemonChecking" class="empty-state">连接中...</div>
    </main>
  </div>
</template>

<style scoped>
.settings-btn {
  background: none;
  border: none;
  font-size: 18px;
  cursor: pointer;
  color: var(--text-secondary);
  padding: 2px 8px;
  border-radius: 4px;
}
.settings-btn:hover { background: var(--bg-hover); }

.sidebar-warn { font-size: 13px; padding: 16px !important; flex: 0 0 auto !important; }

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
}
.daemon-banner code {
  background: rgba(0,0,0,0.06);
  padding: 2px 6px;
  border-radius: 3px;
  font-size: 12px;
}
.retry-btn {
  background: none;
  border: 1px solid #664d03;
  color: #664d03;
  padding: 2px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}
.retry-btn:hover { background: rgba(0,0,0,0.06); }
</style>
