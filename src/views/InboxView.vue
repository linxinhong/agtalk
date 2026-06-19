<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from "vue";
import {
  listInbox,
  getMessage,
  getAttachmentContent,
  markRead,
  markDone,
  replyApproval,
} from "../lib/ipc";
import type { InboxItem, Message } from "../lib/types";

const props = defineProps<{ participant: string }>();

type FilterKey = "pending" | "action_required" | "unread" | "all";

const filters: { key: FilterKey; label: string; status: string | null }[] = [
  { key: "pending", label: "待处理", status: null },
  { key: "action_required", label: "需操作", status: "action_required" },
  { key: "unread", label: "未读", status: "unread" },
  { key: "all", label: "全部未完成", status: "all" },
];

const items = ref<InboxItem[]>([]);
const loading = ref(false);
const currentFilter = ref<FilterKey>("pending");
const selectedItem = ref<InboxItem | null>(null);
const detail = ref<Message | null>(null);
const expandedBodies = ref<Record<string, string>>({});
const error = ref<string | null>(null);

let pollTimer: number | undefined;

const currentStatus = computed(() => {
  const f = filters.find((x) => x.key === currentFilter.value);
  return f?.status ?? null;
});

async function loadInbox() {
  if (!props.participant) return;
  loading.value = true;
  error.value = null;
  try {
    items.value = await listInbox(
      props.participant,
      currentStatus.value,
      50,
      true
    );
    if (selectedItem.value) {
      const updated = items.value.find((i) => i.id === selectedItem.value!.id);
      if (!updated) {
        // 原 item 已不在当前列表（如被 mark done），清空选择
        selectedItem.value = null;
        detail.value = null;
      }
    }
  } catch (e) {
    console.error("加载 inbox 失败:", e);
    error.value = "加载 inbox 失败";
  } finally {
    loading.value = false;
  }
}

async function selectItem(item: InboxItem) {
  selectedItem.value = item;
  detail.value = null;
  expandedBodies.value = {};
  try {
    detail.value = await getMessage(item.id, props.participant);
    if (item.delivery.status !== "read" && item.delivery.status !== "done") {
      await markRead(item.id, props.participant);
      await loadInbox();
    }
  } catch (e) {
    console.error("加载消息详情失败:", e);
  }
}

function parseChoices(msg: Message): string[] {
  try {
    const meta = JSON.parse(msg.metadata || "{}");
    return meta.choices || [];
  } catch {
    return [];
  }
}

async function handleReply(choice: string) {
  if (!detail.value) return;
  try {
    await replyApproval(
      detail.value.id,
      choice,
      undefined,
      props.participant
    );
    await markDone(detail.value.id, props.participant);
    detail.value = null;
    selectedItem.value = null;
    await loadInbox();
  } catch (e) {
    console.error("审批回复失败:", e);
  }
}

async function handleDone() {
  if (!detail.value) return;
  try {
    await markDone(detail.value.id, props.participant);
    detail.value = null;
    selectedItem.value = null;
    await loadInbox();
  } catch (e) {
    console.error("标记完成失败:", e);
  }
}

async function expandFullBody() {
  if (!detail.value) return;
  const msg = detail.value;
  if (msg.full_body) {
    expandedBodies.value[msg.id] = msg.full_body;
    return;
  }
  const att = msg.attachments.find((a) => a.role === "full_body");
  if (att) {
    try {
      expandedBodies.value[msg.id] = await getAttachmentContent(
        att.id,
        props.participant
      );
    } catch (e) {
      console.error("加载全文失败:", e);
    }
  }
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTime(ts: string): string {
  const d = new Date(ts);
  return d.toLocaleString("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

onMounted(() => {
  loadInbox();
  pollTimer = window.setInterval(loadInbox, 3000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});

watch(
  () => props.participant,
  () => {
    selectedItem.value = null;
    detail.value = null;
    loadInbox();
  }
);

watch(currentFilter, () => {
  selectedItem.value = null;
  detail.value = null;
  loadInbox();
});
</script>

<template>
  <div class="inbox-layout">
    <!-- 左侧列表 -->
    <aside class="inbox-sidebar">
      <div class="inbox-filters">
        <button
          v-for="f in filters"
          :key="f.key"
          class="filter-btn"
          :class="{ active: currentFilter === f.key }"
          @click="currentFilter = f.key"
        >
          {{ f.label }}
        </button>
      </div>

      <div class="inbox-list">
        <div v-if="loading && items.length === 0" class="list-empty">加载中...</div>
        <div
          v-for="item in items"
          :key="item.id"
          class="inbox-item"
          :class="{
            active: selectedItem?.id === item.id,
            unread: item.delivery.status !== 'read' && item.delivery.status !== 'done',
            high: item.priority === 'high',
          }"
          @click="selectItem(item)"
        >
          <div class="item-row">
            <span class="item-sender">{{ item.from.name }}</span>
            <span class="item-kind">{{ item.kind }}</span>
          </div>
          <div class="item-subject">
            {{ item.subject || item.content.body.split("\n")[0] || "(无主题)" }}
          </div>
          <div class="item-preview">{{ item.content.body }}</div>
          <div class="item-meta">
            <span class="item-status" :class="item.delivery.status">{{ item.delivery.status }}</span>
            <span v-if="item.content.truncated || item.attachments.length > 0" class="item-attachments">
              {{ item.attachments.length > 0 ? `${item.attachments.length} 附件` : "长文" }}
            </span>
          </div>
        </div>
        <div v-if="!loading && items.length === 0" class="list-empty">
          没有符合条件的消息
        </div>
      </div>
    </aside>

    <!-- 右侧详情 -->
    <main class="inbox-detail">
      <div v-if="!detail" class="detail-empty">
        <div v-if="error">{{ error }}</div>
        <div v-else>选择左侧消息查看详情</div>
      </div>

      <template v-else>
        <div class="detail-header">
          <div class="detail-from">
            {{ detail.sender_name }}
            <span class="detail-kind">{{ detail.content_type }}</span>
          </div>
          <div class="detail-subject">{{ detail.subject || "(无主题)" }}</div>
          <div class="detail-time">{{ formatTime(detail.created_at) }}</div>
        </div>

        <div class="detail-body">
          <pre>{{ expandedBodies[detail.id] || detail.body }}</pre>
          <button
            v-if="
              detail.attachments.some((a) => a.role === 'full_body') &&
              !expandedBodies[detail.id]
            "
            class="text-btn"
            @click="expandFullBody"
          >
            查看全文
          </button>
        </div>

        <div v-if="detail.attachments.length > 0" class="detail-attachments">
          <div class="attachments-title">附件</div>
          <div
            v-for="att in detail.attachments.filter((a) => a.role !== 'full_body')"
            :key="att.id"
            class="attachment-row"
          >
            <span class="attachment-name">{{ att.filename }}</span>
            <span class="attachment-size">{{ formatBytes(att.size) }}</span>
          </div>
        </div>

        <div class="detail-actions">
          <template v-if="detail.content_type === 'approval_request'">
            <button
              v-for="choice in parseChoices(detail)"
              :key="choice"
              class="action-btn primary"
              @click="handleReply(choice)"
            >
              {{ choice }}
            </button>
          </template>
          <button class="action-btn" @click="handleDone">标记完成</button>
        </div>
      </template>
    </main>
  </div>
</template>

<style scoped>
.inbox-layout {
  display: flex;
  width: 100%;
  height: 100%;
}

.inbox-sidebar {
  width: 320px;
  min-width: 260px;
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  background: var(--bg-sidebar);
}

.inbox-filters {
  display: flex;
  gap: 4px;
  padding: 8px;
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}

.filter-btn {
  flex: 1;
  min-width: 56px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text-secondary);
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s;
}

.filter-btn:hover {
  background: var(--bg-hover);
}

.filter-btn.active {
  background: var(--bg-active);
  color: var(--text);
  border-color: var(--border);
  font-weight: 500;
}

.inbox-list {
  flex: 1;
  overflow-y: auto;
}

.inbox-item {
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.12s;
}

.inbox-item:hover {
  background: var(--bg-hover);
}

.inbox-item.active {
  background: var(--bg-active);
}

.inbox-item.unread .item-subject {
  font-weight: 600;
}

.inbox-item.high {
  border-left: 3px solid var(--accent-orange, #ff9500);
}

.item-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}

.item-sender {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}

.item-kind {
  font-size: 11px;
  text-transform: uppercase;
  color: var(--text-secondary);
  background: var(--bg-hover);
  padding: 1px 5px;
  border-radius: 4px;
}

.item-time {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-secondary);
}

.item-subject {
  font-size: 13px;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-bottom: 3px;
}

.item-preview {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-bottom: 5px;
}

.item-meta {
  display: flex;
  gap: 8px;
  font-size: 11px;
}

.item-status {
  text-transform: uppercase;
  color: var(--text-secondary);
}

.item-status.pending,
.item-status.delivered {
  color: var(--accent-orange, #ff9500);
}

.item-status.read {
  color: var(--text-secondary);
}

.item-status.done {
  color: var(--accent-green, #34c759);
}

.item-attachments {
  color: var(--text-tertiary);
}

.list-empty,
.detail-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 14px;
}

.inbox-detail {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  padding: 20px;
}

.detail-header {
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border);
}

.detail-from {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.detail-kind {
  font-size: 11px;
  text-transform: uppercase;
  color: var(--text-secondary);
  background: var(--bg-hover);
  padding: 1px 6px;
  border-radius: 4px;
}

.detail-subject {
  font-size: 16px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
}

.detail-time {
  font-size: 12px;
  color: var(--text-secondary);
}

.detail-body {
  flex: 1;
  font-size: 14px;
  line-height: 1.6;
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  margin-bottom: 16px;
}

.detail-body pre {
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
  margin: 0;
}

.text-btn {
  margin-top: 8px;
  padding: 4px 10px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text-secondary);
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
}

.text-btn:hover {
  background: var(--bg-hover);
}

.detail-attachments {
  margin-bottom: 16px;
}

.attachments-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 6px;
}

.attachment-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-bottom: 6px;
  font-size: 12px;
  background: var(--bg-sidebar);
}

.attachment-name {
  color: var(--text);
}

.attachment-size {
  color: var(--text-secondary);
}

.detail-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.action-btn {
  padding: 7px 16px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text);
  border-radius: 6px;
  cursor: pointer;
  font-size: 13px;
  transition: background 0.12s;
}

.action-btn:hover {
  background: var(--bg-hover);
}

.action-btn.primary {
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
}

.action-btn.primary:hover {
  opacity: 0.9;
}
</style>
