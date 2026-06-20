<script setup lang="ts">
import { ref, onMounted } from "vue";
import { getMessage, replyApproval } from "../lib/ipc";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Message } from "../lib/types";

const props = defineProps<{ msgId: string }>();

const msg = ref<Message | null>(null);
const loading = ref(true);
const error = ref("");
const submitting = ref(false);

const HUMAN_PARTICIPANT = "human";

onMounted(async () => {
  try {
    if (!props.msgId) {
      throw new Error("缺少审批消息 ID");
    }
    msg.value = await getMessage(props.msgId, HUMAN_PARTICIPANT);
  } catch (e) {
    error.value = String(e);
  }
  loading.value = false;

  // 弹窗关闭反馈由 daemon 端 ChildMonitor 监控子进程退出实现，
  // 无需前端拦截窗口关闭事件。
});

function parseChoices(m: Message): string[] {
  try {
    const meta = JSON.parse(m.metadata || "{}");
    return meta.choices || [];
  } catch {
    return [];
  }
}

async function handleReply(choice: string) {
  if (!msg.value || submitting.value) return;
  submitting.value = true;
  error.value = "";
  try {
    await replyApproval(props.msgId, choice, undefined, HUMAN_PARTICIPANT);
    await getCurrentWindow().close();
  } catch (e) {
    error.value = `审批失败: ${e}`;
    submitting.value = false;
  }
}
</script>

<template>
  <div class="approval-popup">
    <div v-if="loading" class="state">加载中...</div>
    <div v-else-if="error" class="state error">{{ error }}</div>
    <template v-else-if="msg">
      <div class="approval-from">来自 {{ msg.sender_name }}</div>
      <div class="approval-q">{{ msg.body }}</div>
      <div class="approval-choices">
        <button
          v-for="c in parseChoices(msg)"
          :key="c"
          :disabled="submitting"
          @click="handleReply(c)"
        >{{ c }}</button>
      </div>
      <div v-if="parseChoices(msg).length === 0" class="state error">无可用选项，请检查消息元数据</div>
    </template>
  </div>
</template>

<style scoped>
.approval-popup {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 24px;
  gap: 14px;
  background: var(--bg);
  color: var(--text);
}
.approval-from {
  font-size: 12px;
  color: var(--text-secondary);
}
.approval-q {
  font-size: 16px;
  font-weight: 600;
  line-height: 1.5;
  flex: 1;
}
.approval-choices {
  display: flex;
  gap: 10px;
  justify-content: flex-end;
}
.approval-choices button {
  padding: 8px 22px;
  border: none;
  border-radius: var(--radius);
  background: var(--accent);
  color: white;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
}
.approval-choices button:hover { opacity: 0.85; }
.approval-choices button:disabled { opacity: 0.5; cursor: default; }
.state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
}
.state.error { color: var(--danger); }
</style>
