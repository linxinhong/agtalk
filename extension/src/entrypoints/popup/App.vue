<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { lookup } from '@/api';
import type { JoinResult, Mailbox, Message, StoredSession } from '@/types';

const joined = ref(false);
const session = ref<StoredSession | null>(null);
const messages = ref<Message[]>([]);
const baseUrl = ref('');
const defaultBaseUrl = ref('');
const editingBaseUrl = ref('');

const name = ref('');
const intro = ref('browser');

const to = ref('');
const body = ref('');
const candidates = ref<Mailbox[]>([]);
const lookupName = ref('');

onMounted(async () => {
  await refreshState();
});

function short(id: string): string {
  return id ? id.slice(0, 8) : '';
}

async function refreshState() {
  const resp = await browser.runtime.sendMessage({ action: 'GET_STATE' });
  if (resp.session) {
    joined.value = true;
    session.value = resp.session as StoredSession;
    messages.value = (resp.messages as Message[]) || [];
  } else {
    joined.value = false;
    session.value = null;
    messages.value = [];
  }
  baseUrl.value = resp.baseUrl || '';
  defaultBaseUrl.value = resp.defaultBaseUrl || '';
  editingBaseUrl.value = baseUrl.value;
}

async function saveBaseUrl() {
  await browser.runtime.sendMessage({
    action: 'SET_BASE_URL',
    url: editingBaseUrl.value,
  });
  baseUrl.value = editingBaseUrl.value;
}

async function resetBaseUrl() {
  await browser.runtime.sendMessage({ action: 'RESET_BASE_URL' });
  editingBaseUrl.value = defaultBaseUrl.value;
  baseUrl.value = defaultBaseUrl.value;
}

async function doJoin() {
  const resp = await browser.runtime.sendMessage({
    action: 'JOIN',
    name: name.value,
    intro: intro.value,
  });
  if (resp.ok) {
    const result = resp.session as JoinResult;
    joined.value = true;
    session.value = {
      address: result.address,
      token: result.token,
      name: result.name,
    };
    messages.value = [];
  } else {
    alert(resp.error || 'join failed');
  }
}

async function doLeave() {
  await browser.runtime.sendMessage({ action: 'LEAVE' });
  joined.value = false;
  session.value = null;
  messages.value = [];
}

async function doSend() {
  if (!session.value || !to.value || !body.value) return;
  const resp = await browser.runtime.sendMessage({
    action: 'SEND',
    to: to.value,
    body: body.value,
  });
  if (resp.ok) {
    body.value = '';
  } else {
    alert(resp.error || 'send failed');
  }
}

async function doLookup() {
  candidates.value = await lookup(lookupName.value || undefined);
}

function useCandidate(addr: string) {
  to.value = addr;
}
</script>

<template>
  <div class="popup">
    <h1>agtalk</h1>

    <div class="base-url">
      <label>daemon</label>
      <input v-model="editingBaseUrl" :placeholder="defaultBaseUrl" />
      <div class="base-url-actions">
        <button @click="saveBaseUrl">Save</button>
        <button v-if="baseUrl !== defaultBaseUrl" @click="resetBaseUrl" class="reset">Reset</button>
      </div>
      <p class="hint">current: {{ baseUrl }}</p>
    </div>

    <div v-if="!joined" class="join-form">
      <label>name</label>
      <input v-model="name" placeholder="留空自动生成 browser-<short>" />
      <label>intro</label>
      <input v-model="intro" />
      <button @click="doJoin">Join</button>
    </div>

    <div v-else class="main">
      <div class="session">
        <p>
          <strong>{{ session?.name || 'browser' }}</strong>
          <span class="addr">{{ short(session?.address || '') }}</span>
        </p>
        <button @click="doLeave">Leave</button>
      </div>

      <div class="lookup">
        <input v-model="lookupName" placeholder="lookup name" />
        <button @click="doLookup">Lookup</button>
        <ul>
          <li v-for="c in candidates" :key="c.address" @click="useCandidate(c.address)">
            <strong>{{ c.name }}</strong>
            <span class="notify" :class="{ off: !c.notify_ready }">notify={{ c.notify }}</span>
            <span class="addr">{{ short(c.address) }}</span>
          </li>
        </ul>
      </div>

      <div class="send">
        <input v-model="to" placeholder="to address" />
        <input v-model="body" placeholder="body" @keyup.enter="doSend" />
        <button @click="doSend">Send</button>
      </div>

      <div class="messages">
        <h2>Messages</h2>
        <ul>
          <li v-for="m in messages.slice().reverse()" :key="m.id">
            <strong>{{ m.from_name }}</strong>: {{ m.body }}
          </li>
        </ul>
      </div>
    </div>
  </div>
</template>

<style>
.popup {
  width: 360px;
  padding: 12px;
  font-family: system-ui, -apple-system, sans-serif;
}
h1 {
  margin: 0 0 12px;
  font-size: 18px;
}
.base-url {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 12px;
  padding: 8px;
  background: #f5f5f5;
  border-radius: 4px;
}
.base-url-actions {
  display: flex;
  gap: 6px;
}
.base-url .hint {
  margin: 0;
  font-size: 11px;
  color: #666;
}
button.reset {
  background: #666;
}
.join-form,
.send,
.lookup,
.session {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 12px;
}
input {
  padding: 6px;
  border: 1px solid #ccc;
  border-radius: 4px;
}
button {
  padding: 6px 10px;
  background: #111;
  color: #fff;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}
.messages ul,
.lookup ul {
  list-style: none;
  padding: 0;
  margin: 0;
  max-height: 140px;
  overflow-y: auto;
}
.messages li,
.lookup li {
  padding: 4px 0;
  border-bottom: 1px solid #eee;
  cursor: pointer;
}
.addr {
  margin-left: 6px;
  color: #666;
  font-size: 11px;
  font-weight: normal;
}
.notify {
  margin-left: 6px;
  color: #0a0;
  font-size: 11px;
  font-weight: normal;
}
.notify.off {
  color: #999;
}
</style>
