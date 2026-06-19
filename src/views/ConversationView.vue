<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from "vue";
import {
  listConversations,
  getMessages,
  getAttachmentContent,
  sendMessage,
  markDone,
  replyApproval,
} from "../lib/ipc";
import type { Conversation, Message } from "../lib/types";

const props = defineProps<{ daemonOnline: boolean }>();

const conversations = ref<Conversation[]>([]);
const messages = ref<Message[]>([]);
const activeConvId = ref<string | null>(null);
const loading = ref(false);
const replyText = ref("");
const currentParticipant = ref("me");
const expandedBodies = ref<Record<string, string>>({});

let pollTimer: number | undefined;

onMounted(async () => {
  await loadConversations();
  pollTimer = window.setInterval(async () => {
    await loadConversations();
    if (activeConvId.value) {
      try {
        messages.value = await getMessages(activeConvId.value);
      } catch { /* ignore poll errors */ }
    }
  }, 3000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});

async function loadConversations() {
  try {
    conversations.value = await listConversations("me");
  } catch (e) {
    console.error("加载对话列表失败:", e);
  }
}

async function selectConversation(id: string) {
  activeConvId.value = id;
  loading.value = true;
  try {
    messages.value = await getMessages(id);
  } catch (e) {
    console.error("加载消息失败:", e);
    messages.value = [];
  }
  loading.value = false;
  await nextTick();
  scrollToBottom();
}

async function handleSend() {
  const text = replyText.value.trim();
  if (!text || !activeConvId.value) return;

  const conv = conversations.value.find((c) => c.id === activeConvId.value);
  if (!conv) return;

  const other = conv.participants.find(
    (p) => p !== currentParticipant.value
  ) || conv.participants[0];

  replyText.value = "";
  try {
    await sendMessage(other, text, activeConvId.value, undefined, undefined, currentParticipant.value);
    await selectConversation(activeConvId.value);
    await loadConversations();
  } catch (e) {
    console.error("发送失败:", e);
  }
}

async function handleDone(msgId: string) {
  try {
    await markDone(msgId, currentParticipant.value);
    if (activeConvId.value) {
      await selectConversation(activeConvId.value);
    }
    await loadConversations();
  } catch (e) {
    console.error("标记完成失败:", e);
  }
}

async function handleReply(msgId: string, choice: string) {
  try {
    await replyApproval(msgId, choice);
    if (activeConvId.value) {
      messages.value = await getMessages(activeConvId.value);
    }
    await loadConversations();
  } catch (e) {
    console.error("审批回复失败:", e);
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

function findReply(msg: Message): Message | undefined {
  return messages.value.find(
    (m) => m.reply_to_id === msg.id && m.content_type === "approval_response"
  );
}

function isReplied(msg: Message): boolean {
  return !!findReply(msg);
}

function repliedChoice(msg: Message): string {
  const reply = findReply(msg);
  if (!reply) return "";
  try {
    const meta = JSON.parse(reply.metadata || "{}");
    return meta.choice || reply.body;
  } catch {
    return reply.body;
  }
}

async function expandFullBody(msg: Message) {
  if (msg.full_body) {
    expandedBodies.value[msg.id] = msg.full_body;
    return;
  }
  const fullBodyAttachment = msg.attachments.find((a) => a.role === "full_body");
  if (fullBodyAttachment) {
    try {
      expandedBodies.value[msg.id] = await getAttachmentContent(fullBodyAttachment.id);
    } catch (e) {
      console.error("加载全文失败:", e);
    }
  }
}

function scrollToBottom() {
  const el = document.querySelector(".message-list");
  if (el) el.scrollTop = el.scrollHeight;
}

function formatTime(ts: string): string {
  const d = new Date(ts);
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    handleSend();
  }
}
</script>

<template>
  <div class="app-layout">
    <!-- 对话列表 -->
    <aside class="sidebar">
      <div class="sidebar-header">
        <span>对话</span>
      </div>
      <div class="sidebar-list">
        <div
          v-for="conv in conversations"
          :key="conv.id"
          class="conversation-item"
          :class="{ active: conv.id === activeConvId }"
          @click="selectConversation(conv.id)"
        >
          <div class="conv-title">{{ conv.title || conv.participants.join(", ") }}</div>
          <div class="conv-preview" v-if="conv.last_message">
            {{ conv.last_message.sender_name }}: {{ conv.last_message.body }}
          </div>
          <div class="conv-meta">
            <span class="conv-time" v-if="conv.last_message">
              {{ formatTime(conv.last_message.created_at) }}
            </span>
            <span class="conv-badge" v-if="conv.unread_count > 0">
              {{ conv.unread_count }}
            </span>
          </div>
        </div>
        <div v-if="conversations.length === 0" class="empty-state" style="padding: 32px;">
          暂无对话
        </div>
      </div>
    </aside>

    <!-- 消息区 -->
    <main class="main-area">
      <template v-if="activeConvId && props.daemonOnline">
        <div class="message-header">
          {{ conversations.find(c => c.id === activeConvId)?.title || activeConvId }}
        </div>
        <div class="message-list" v-if="!loading">
          <div
            v-for="msg in messages"
            :key="msg.id"
            class="message-bubble"
            :class="msg.sender_name === currentParticipant ? 'self' : 'other'"
          >
            <div class="msg-sender" v-if="msg.sender_name !== currentParticipant">
              {{ msg.sender_name }}
            </div>

            <!-- 审批卡片 -->
            <div v-if="msg.content_type === 'approval_request'" class="approval-card">
              <div class="approval-q">{{ msg.body }}</div>
              <div class="approval-choices" v-if="!isReplied(msg) && parseChoices(msg).length">
                <button
                  v-for="c in parseChoices(msg)"
                  :key="c"
                  @click="handleReply(msg.id, c)"
                  :disabled="isReplied(msg)"
                >{{ c }}</button>
              </div>
              <div v-if="isReplied(msg)" class="approval-done">
                已选：{{ repliedChoice(msg) }}
              </div>
            </div>

            <!-- 普通消息 -->
            <div v-else class="msg-body">
              <pre style="white-space: pre-wrap; word-break: break-word; margin: 0; font-family: inherit;">{{ expandedBodies[msg.id] || msg.body }}</pre>
              <button
                v-if="msg.attachments.some((a) => a.role === 'full_body') && !expandedBodies[msg.id]"
                @click="expandFullBody(msg)"
                class="expand-btn"
              >
                查看全文
              </button>
            </div>

            <div class="msg-time">
              {{ formatTime(msg.created_at) }}
              <span
                class="msg-status"
                v-if="msg.sender_name !== currentParticipant"
                @click="handleDone(msg.id)"
                style="cursor: pointer; margin-left: 4px;"
              >
                {{ msg.recipients[0]?.status === 'done' ? '✓✓' : '✓' }}
              </span>
            </div>
          </div>
        </div>
        <div v-else class="empty-state">加载中...</div>
        <div class="reply-area">
          <textarea
            v-model="replyText"
            placeholder="输入消息..."
            @keydown="handleKeydown"
            rows="1"
          ></textarea>
          <button @click="handleSend" :disabled="!replyText.trim()">发送</button>
        </div>
      </template>
      <div v-else-if="!props.daemonOnline" class="empty-state">daemon 离线</div>
      <div v-else class="empty-state">选择一个对话开始</div>
    </main>
  </div>
</template>
