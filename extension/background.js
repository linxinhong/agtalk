// MV3 Service Worker：agtalk Web Bridge 后台核心
// 负责自动 join/attach、inbox 轮询、content 脚本消息转发
import { AgtalkClient } from './agtalk-client.js';
import { MessageStore } from './lib/storage.mjs';

const PLATFORM_SCRIPTS = [
  'platforms/chatgpt.js',
  'platforms/claude.js',
  'platforms/sider.js',
  'platforms/chatglm.js',
  'platforms/custom.js',
  'content.js',
];

const DEFAULT_CONFIG = {
  daemonUrl: 'http://127.0.0.1:19527',
  agtalkUrl: 'http://127.0.0.1:19527',
  agentName: '',
  agentRole: 'web',
  agentBio: 'Web AI bridge participant',
  agentCapabilities: '',
  targetAgent: '',
  enabled: true,
  autoForward: false,
  autoReceive: true,
  autoInject: false,
  connectedPeers: [],
  autoInjectPeers: [],
  enableChatgpt: true,
  enableClaude: true,
  enableSider: true,
  enableChatglm: true,
  enableCustom: false,
  pollInterval: 5000,
  workspaceRoot: '/virtual/web-bridge',
  workspaceName: 'web-bridge',
  captureDelay: 300,
};
let runtimeConfig = { ...DEFAULT_CONFIG };
let agtalkClient = null;
let inboxPollTimer = null;
let lastInboxIds = new Set();
let inboxInitialized = false;
let connectionState = { connected: false, error: null, reconnecting: false };
let connectionWatchTimer = null;
let reconnectAttempt = 0;
let tabPeerHints = {};
let initResolve = null;
const initPromise = new Promise((resolve) => { initResolve = resolve; });

// 加载配置与 session
chrome.storage.local.get(['agtalk_config', 'agtalk_session'], (result) => {
  if (result.agtalk_config) {
    runtimeConfig = normalizeConfig({ ...DEFAULT_CONFIG, ...result.agtalk_config });
  }
  loadTabPeerHints();
  initAgtalkClient(result.agtalk_session).then(() => {
    initResolve();
  }).catch((err) => {
    console.error('[BG] 初始化失败:', err.message);
    initResolve();
  });
});

chrome.storage.onChanged.addListener((changes) => {
  if (changes.agtalk_config) {
    runtimeConfig = normalizeConfig({ ...DEFAULT_CONFIG, ...changes.agtalk_config.newValue });
    reconcileInboxPolling();
  }
  if (changes.agtalk_tab_peer_hints) {
    tabPeerHints = normalizeTabPeerHints(changes.agtalk_tab_peer_hints.newValue);
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  const key = String(tabId);
  if (!tabPeerHints[key]) return;
  delete tabPeerHints[key];
  persistTabPeerHints().catch(() => {});
});

// ─── Config normalization ───
function normalizeConfig(config) {
  const merged = { ...DEFAULT_CONFIG, ...config };
  merged.activePeer = String(merged.activePeer || '');
  const connected = Array.isArray(merged.connectedPeers)
    ? merged.connectedPeers.map((p) => String(p).trim()).filter(Boolean)
    : [];
  const legacyTarget = merged.targetAgent ? String(merged.targetAgent).trim() : '';
  if (legacyTarget && !connected.includes(legacyTarget)) connected.unshift(legacyTarget);
  merged.connectedPeers = Array.from(new Set(connected));
  if (merged.activePeer && !merged.connectedPeers.includes(merged.activePeer)) {
    merged.activePeer = '';
  }
  if (!merged.activePeer && merged.connectedPeers.length > 0) {
    merged.activePeer = merged.connectedPeers[0];
    merged.targetAgent = merged.activePeer;
  }
  return merged;
}

// ─── Tab-Peer 关联系统 ───
function loadTabPeerHints() {
  chrome.storage.local.get(['agtalk_tab_peer_hints'], (result) => {
    tabPeerHints = normalizeTabPeerHints(result.agtalk_tab_peer_hints);
  });
}

function normalizeTabPeerHints(hints) {
  if (!hints || typeof hints !== 'object' || Array.isArray(hints)) return {};
  const out = {};
  for (const [tabId, value] of Object.entries(hints)) {
    if (!value || typeof value !== 'object') continue;
    const peer = String(value.peer || '').trim();
    const autoReplyPeer = String(value.autoReplyPeer || '').trim();
    if (!peer && !autoReplyPeer) continue;
    out[String(tabId)] = {
      peer,
      autoReplyPeer,
      url: String(value.url || ''),
      updated_at: Number(value.updated_at || Date.now()),
    };
  }
  return out;
}

async function persistTabPeerHints() {
  return new Promise((resolve) => {
    chrome.storage.local.set({ agtalk_tab_peer_hints: tabPeerHints }, resolve);
  });
}

function getConnectedPeers() {
  return Array.isArray(runtimeConfig.connectedPeers) ? runtimeConfig.connectedPeers : [];
}

function getRecommendedPeerForTab(tabId) {
  const connected = new Set(getConnectedPeers());
  const hint = tabId != null ? tabPeerHints[String(tabId)] : null;
  if (hint?.peer && connected.has(hint.peer)) return hint.peer;
  if (runtimeConfig.activePeer && connected.has(runtimeConfig.activePeer)) return runtimeConfig.activePeer;
  return getConnectedPeers()[0] || '';
}

async function associateTabPeer(tabId, peer, url = '') {
  if (tabId == null) return;
  const key = String(tabId);
  const existing = tabPeerHints[key] || {};
  if (!peer) {
    if (existing.autoReplyPeer) {
      tabPeerHints = {
        ...tabPeerHints,
        [key]: { ...existing, peer: '', url: String(url || existing.url || ''), updated_at: Date.now() },
      };
      await persistTabPeerHints();
    } else if (tabPeerHints[key]) {
      const next = { ...tabPeerHints };
      delete next[key];
      tabPeerHints = next;
      await persistTabPeerHints();
    }
    return;
  }
  tabPeerHints = {
    ...tabPeerHints,
    [key]: {
      ...existing,
      peer: String(peer).trim(),
      url: String(url || existing.url || ''),
      updated_at: Date.now(),
    },
  };
  await persistTabPeerHints();
}

async function setAutoReplyPeer(tabId, peer, url = '') {
  if (tabId == null) return;
  const key = String(tabId);
  const existing = tabPeerHints[key] || {};
  if (!peer) {
    if (existing.peer) {
      tabPeerHints = {
        ...tabPeerHints,
        [key]: { ...existing, autoReplyPeer: '', url: String(url || existing.url || ''), updated_at: Date.now() },
      };
      await persistTabPeerHints();
    } else if (tabPeerHints[key]) {
      const next = { ...tabPeerHints };
      delete next[key];
      tabPeerHints = next;
      await persistTabPeerHints();
    }
    return;
  }
  tabPeerHints = {
    ...tabPeerHints,
    [key]: {
      ...existing,
      autoReplyPeer: String(peer).trim(),
      url: String(url || existing.url || ''),
      updated_at: Date.now(),
    },
  };
  await persistTabPeerHints();
}

function getAutoReplyPeer(tabId) {
  if (tabId == null) return '';
  return tabPeerHints[String(tabId)]?.autoReplyPeer || '';
}

function isConnectedPeer(name) {
  return !!name && getConnectedPeers().includes(name);
}

function resolveTargetPeer(explicit) {
  if (explicit && isConnectedPeer(explicit)) return explicit;
  const target = runtimeConfig.activePeer || runtimeConfig.targetAgent || '';
  if (target && isConnectedPeer(target)) return target;
  return getConnectedPeers()[0] || '';
}

function getDaemonUrl() {
  return runtimeConfig.daemonUrl || runtimeConfig.agtalkUrl || 'http://127.0.0.1:19527';
}

async function initAgtalkClient(savedSession) {
  agtalkClient = new AgtalkClient(getDaemonUrl());
  startConnectionWatch();
  if (savedSession?.session_id && savedSession?.token) {
    try {
      await agtalkClient.auth(savedSession.session_id, savedSession.token);
      console.log('[BG] agtalk session 认证成功');
      connectionState = { connected: true, error: null, reconnecting: false };
      startInboxPollingIfNeeded();
      return;
    } catch (err) {
      console.log('[BG] session 已失效，准备重新 join:', err.message);
    }
  }
  await ensureJoined();
}

async function ensureJoined() {
  if (!runtimeConfig.enabled) return;
  if (agtalkClient?.sessionId) return;
  if (!runtimeConfig.agentName) return;

  agtalkClient = new AgtalkClient(getDaemonUrl());
  try {
    const caps = runtimeConfig.agentCapabilities
      ? runtimeConfig.agentCapabilities.split(/[,，]/).map((s) => s.trim()).filter(Boolean)
      : [];
    const data = await agtalkClient.join({
      workspaceRoot: runtimeConfig.workspaceRoot,
      workspaceName: runtimeConfig.workspaceName,
      name: runtimeConfig.agentName,
      participantType: 'web',
      role: runtimeConfig.agentRole || 'web',
      intro: runtimeConfig.agentBio || '',
      capabilities: caps,
      transport: 'http',
      takeover: true,
    });
    await saveSession(data);
    connectionState = { connected: true, error: null, reconnecting: false };
    reconnectAttempt = 0;
    console.log('[BG] agtalk join 成功:', data.session_id);
    startInboxPollingIfNeeded();
  } catch (err) {
    connectionState = { connected: false, error: err.message, reconnecting: false };
    console.error('[BG] agtalk join 失败:', err.message);
  }
}

function saveSession(data) {
  const session = {
    session_id: data.session_id,
    token: data.token,
    participant: data.participant,
    workspace_id: data.workspace_id,
  };
  return new Promise((resolve) => {
    chrome.storage.local.set({ agtalk_session: session }, resolve);
  });
}

function startConnectionWatch() {
  if (connectionWatchTimer) return;
  checkConnection();
  connectionWatchTimer = setInterval(checkConnection, 10000);
}

function stopConnectionWatch() {
  if (connectionWatchTimer) {
    clearInterval(connectionWatchTimer);
    connectionWatchTimer = null;
  }
}

async function checkConnection() {
  const url = getDaemonUrl();
  const client = agtalkClient || new AgtalkClient(url);
  try {
    const connected = await client.ping();
    if (connected) {
      if (!connectionState.connected) {
        connectionState = { connected: true, error: null, reconnecting: false };
        reconnectAttempt = 0;
        console.log('[BG] daemon 连接恢复');
        // 如果已启用且有 agentName，确保已 join
        if (runtimeConfig.enabled && runtimeConfig.agentName && !agtalkClient?.sessionId) {
          await ensureJoined();
        }
      }
    } else {
      throw new Error('无法 ping 通 daemon');
    }
  } catch (err) {
    if (connectionState.connected || !connectionState.error) {
      connectionState = { connected: false, error: err.message, reconnecting: false };
      console.error('[BG] daemon 连接断开:', err.message);
    }
    scheduleReconnect();
  }
}

function scheduleReconnect() {
  if (connectionState.reconnecting) return;
  if (!runtimeConfig.enabled || !runtimeConfig.agentName) return;
  connectionState.reconnecting = true;
  reconnectAttempt++;
  const delay = Math.min(1000 * Math.pow(2, reconnectAttempt - 1), 30000);
  console.log(`[BG] ${delay}ms 后尝试第 ${reconnectAttempt} 次重连`);
  setTimeout(async () => {
    try {
      await ensureJoined();
      const connected = await agtalkClient?.ping();
      if (connected) {
        connectionState = { connected: true, error: null, reconnecting: false };
        reconnectAttempt = 0;
        console.log('[BG] 重连成功');
      } else {
        throw new Error('重连后 ping 失败');
      }
    } catch (err) {
      connectionState = { connected: false, error: err.message, reconnecting: false };
      console.error('[BG] 重连失败:', err.message);
    }
  }, delay);
}

function reconcileInboxPolling() {
  if (inboxPollTimer) {
    clearInterval(inboxPollTimer);
    inboxPollTimer = null;
  }
  startInboxPollingIfNeeded();
}

function startInboxPollingIfNeeded() {
  if (!runtimeConfig.enabled || !runtimeConfig.autoReceive || !agtalkClient?.sessionId) return;
  if (inboxPollTimer) return;
  pollInbox();
  inboxPollTimer = setInterval(pollInbox, runtimeConfig.pollInterval || 5000);
}

async function pollInbox() {
  if (!agtalkClient?.sessionId) return;
  try {
    const items = await agtalkClient.inbox({ participant: runtimeConfig.agentName, status: 'all', limit: 1000, peek: true });
    if (!Array.isArray(items)) return;

    // 首次轮询：只记录已有消息 ID，不注入，避免扩展启动/打开 popup 时把历史消息全部注入
    // 但会把历史消息保存到本地 IndexedDB，方便离线查看
    if (!inboxInitialized) {
      items.forEach((item) => lastInboxIds.add(item.id));
      MessageStore.saveMany(items).catch((err) => console.error('[BG] 保存历史消息失败:', err.message));
      inboxInitialized = true;
      console.log('[BG] inbox 初始化完成，已记录', lastInboxIds.size, '条历史消息');
      return;
    }

    const newItems = items.filter((item) => !lastInboxIds.has(item.id));
    if (newItems.length === 0) return;

    for (const item of newItems) {
      lastInboxIds.add(item.id);
      MessageStore.save(item).catch((err) => console.error('[BG] 保存消息失败:', err.message));
      const fromPeer = item.from?.name || item.from_agent || '';
      const isAutoReply = isAutoReplyMessage(fromPeer);
      const autoInjectPeers = Array.isArray(runtimeConfig.autoInjectPeers) ? runtimeConfig.autoInjectPeers : [];
      const shouldAutoInject = runtimeConfig.autoInject && autoInjectPeers.includes(fromPeer);
      if (shouldAutoInject || isAutoReply) {
        await dispatchIncomingToWebTabs(item);
        MessageStore.markInjected(item.id).catch(() => {});
        // 自动注入后只标记已读，不标记完成，这样消息仍保留在 inbox 中
        await agtalkMarkRead(item.id);
      }
    }
  } catch (err) {
    console.error('[BG] inbox 轮询失败:', err.message);
  }
}

async function ensureContentScriptInjected(tabId, retries = 3) {
  for (let attempt = 1; attempt <= retries; attempt++) {
    const loaded = await new Promise((resolve) => {
      const timer = setTimeout(() => {
        resolve(false);
      }, 2000);
      chrome.tabs.sendMessage(tabId, { type: 'PING' }, (res) => {
        clearTimeout(timer);
        resolve(!chrome.runtime.lastError && res?.pong === true);
      });
    });
    if (loaded) {
      console.log('[BG] content script 已存在，tab:', tabId);
      return;
    }
    if (attempt < retries) {
      console.log('[BG] PING 未响应，等待重试...');
      await new Promise((r) => setTimeout(r, 300));
    }
  }

  console.log('[BG] 动态注入 content script，tab:', tabId);
  await new Promise((resolve, reject) => {
    chrome.scripting.executeScript({ target: { tabId }, files: PLATFORM_SCRIPTS }, (results) => {
      if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
      resolve(results);
    });
  });
}

async function dispatchIncomingToWebTabs(item) {
  const patterns = ['https://chatgpt.com/*', 'https://claude.ai/*', 'https://sider.ai/*', 'https://chatglm.cn/*'];
  const allTabs = await new Promise((resolve) => chrome.tabs.query({ url: patterns }, resolve));
  if (!Array.isArray(allTabs) || allTabs.length === 0) return;

  const message = { type: 'AGTALK_INCOMING', item };
  const fromPeer = item.from?.name || item.from_agent || '';

  // 1. 若消息来源 peer 是某个 tab 的自动回复 Agent，优先注入到该 tab
  if (fromPeer) {
    const autoReplyTabId = findTabIdByAutoReplyPeer(fromPeer, allTabs);
    if (autoReplyTabId != null) {
      try {
        await ensureContentScriptInjected(autoReplyTabId);
        await new Promise((resolve, reject) => {
          chrome.tabs.sendMessage(autoReplyTabId, message, (res) => {
            if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
            resolve(res);
          });
        });
        console.log('[BG] 自动回复消息已注入 tab:', autoReplyTabId, 'from:', fromPeer);
      } catch (err) {
        console.warn('[BG] 注入自动回复 tab 失败:', err.message);
      }
      return;
    }
  }

  // 2. 若消息来源 peer 有关联 tab，优先注入到该 tab
  if (fromPeer) {
    const linkedTabId = findTabIdByPeer(fromPeer, allTabs);
    if (linkedTabId != null) {
      try {
        await ensureContentScriptInjected(linkedTabId);
        await new Promise((resolve, reject) => {
          chrome.tabs.sendMessage(linkedTabId, message, (res) => {
            if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
            resolve(res);
          });
        });
        notifyAutoInject(item);
      } catch (err) {
        console.warn('[BG] 注入关联 tab 失败:', err.message);
      }
      return;
    }
  }

  // 2. 优先注入到当前激活的匹配标签页
  const activeTabs = await new Promise((resolve) => chrome.tabs.query({ active: true, currentWindow: true, url: patterns }, resolve));
  if (Array.isArray(activeTabs) && activeTabs.length > 0) {
    const targetTab = activeTabs[0];
    try {
      await ensureContentScriptInjected(targetTab.id);
      await new Promise((resolve, reject) => {
        chrome.tabs.sendMessage(targetTab.id, message, (res) => {
          if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
          resolve(res);
        });
      });
      // 自动注入后关联 tab -> peer，实现后续自动关联回复
      if (fromPeer) {
        await associateTabPeer(targetTab.id, fromPeer, targetTab.url || '');
      }
      notifyAutoInject(item);
    } catch (err) {
      console.error('[BG] 自动注入当前标签页失败:', err.message);
    }
    return;
  }

  // 3. 回退到所有匹配标签页
  for (const tab of allTabs) {
    chrome.tabs.sendMessage(tab.id, message, () => {
      chrome.runtime.lastError;
    });
    if (fromPeer) {
      associateTabPeer(tab.id, fromPeer, tab.url || '').catch(() => {});
    }
  }
  notifyAutoInject(item);
}

function findTabIdByPeer(peer, tabs) {
  if (!peer || !tabs) return null;
  for (const [tabIdStr, hint] of Object.entries(tabPeerHints)) {
    if (hint.peer === peer) {
      const id = Number(tabIdStr);
      if (tabs.some((t) => t.id === id)) return id;
    }
  }
  return null;
}

function isAutoReplyMessage(fromPeer) {
  if (!fromPeer) return false;
  for (const hint of Object.values(tabPeerHints)) {
    if (hint.autoReplyPeer === fromPeer) return true;
  }
  return false;
}

function findTabIdByAutoReplyPeer(peer, tabs) {
  if (!peer || !tabs) return null;
  for (const [tabIdStr, hint] of Object.entries(tabPeerHints)) {
    if (hint.autoReplyPeer === peer) {
      const id = Number(tabIdStr);
      if (tabs.some((t) => t.id === id)) return id;
    }
  }
  return null;
}

function notifyAutoInject(item) {
  try {
    const from = item.from?.name || item.from_agent || '未知';
    const body = (item.content?.body || item.body || '').slice(0, 80);
    chrome.notifications.create(`agtalk-inject-${item.id}`, {
      type: 'basic',
      iconUrl: 'icons/icon.svg',
      title: `agtalk 消息已自动注入 - ${from}`,
      message: body,
      priority: 1,
    });
  } catch (err) {
    console.error('[BG] 通知创建失败:', err.message);
  }
}

async function forwardToAgtalk(payload) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) throw new Error('agtalk 未连接');

  const toAgent = resolveTargetPeer(payload.toAgent || payload.replyToAgent);
  if (!toAgent) {
    throw new Error('未指定目标 Agent');
  }

  const body = '---\nfrom_agent: ' + (payload.source || runtimeConfig.agentName) +
    '\nconversation_id: ' + (payload.conversation_id || '') +
    '\ntimestamp: ' + (payload.timestamp || Date.now()) +
    '\n---\n' + (payload.turn.assistant || '');

  const data = await agtalkClient.send({
    to: toAgent,
    body,
    conversationId: payload.conversation_id,
    replyTo: payload.replyToMsgId,
    contentType: 'text',
    notify: true,
  });
  const msgId = data?.message?.id || data?.id || ('fwd-' + Date.now());
  console.log('[BG] agtalk 转发成功:', data?.id);
  if (data) {
    MessageStore.save({
      id: msgId,
      chat_id: payload.conversation_id,
      from: { name: runtimeConfig.agentName, type: 'web' },
      recipients: [{ recipient_name: toAgent, status: 'pending' }],
      subject: payload.conversation_id,
      content: { body: payload.turn.assistant },
      content_type: 'text',
      created_at: new Date().toISOString(),
      reply_to_id: payload.replyToMsgId,
    }).catch((err) => console.error('[BG] 保存转发消息失败:', err.message));
  }
  return data;
}

async function agtalkDirectSend(toAgent, body, subject, replyTo) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  toAgent = resolveTargetPeer(toAgent);
  if (!toAgent) return { ok: false, error: '未指定目标 Agent' };

  const data = await agtalkClient.send({
    to: toAgent,
    body: body || '',
    conversationId: subject || null,
    replyTo: replyTo || null,
    contentType: 'text',
    notify: true,
  });
  if (data) {
    const msgId = data?.message?.id || data?.id || ('msg-' + Date.now());
    MessageStore.save({
      id: msgId,
      from: { name: runtimeConfig.agentName, type: 'web' },
      recipients: [{ recipient_name: toAgent, status: 'pending' }],
      subject,
      content: { body },
      content_type: 'text',
      created_at: new Date().toISOString(),
      reply_to_id: replyTo,
    }).catch((err) => console.error('[BG] 保存发送消息失败:', err.message));
  }
  return { ok: true, msg_id: data?.message?.id || data?.id, data };
}

async function agtalkInbox(agent, status) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };

  const target = agent || runtimeConfig.agentName;
  const items = await agtalkClient.inbox({ participant: target, status: status || 'all', limit: 1000, peek: false });
  return { ok: true, agent: target, count: items.length, items };
}

async function checkAgtalkConnection() {
  const url = getDaemonUrl();
  const client = agtalkClient || new AgtalkClient(url);
  try {
    const connected = await client.ping();
    if (!connected) {
      connectionState = { connected: false, url, error: '无法 ping 通 daemon', reconnecting: connectionState.reconnecting };
      return { ...connectionState, url };
    }
    connectionState = { connected: true, error: null, reconnecting: false };
    const stats = await agtalkInboxStats();
    return {
      connected: true,
      url,
      reconnecting: false,
      agent: agtalkClient?.me?.name || runtimeConfig.agentName,
      inboxUnread: stats.unread,
      inboxTotal: stats.total,
      peersOnline: stats.peersOnline,
    };
  } catch (err) {
    connectionState = { connected: false, url, error: err.message, reconnecting: connectionState.reconnecting };
    return { ...connectionState };
  }
}

async function agtalkInboxStats() {
  const fallback = { unread: 0, total: 0, peersOnline: 0 };
  if (!agtalkClient?.sessionId) return fallback;
  try {
    const items = await agtalkClient.inbox({ participant: runtimeConfig.agentName, status: 'all', limit: 1000, peek: true });
    const unread = Array.isArray(items)
      ? items.filter((i) => {
        const delivery = i.delivery || (i.recipients?.[0] ? { status: i.recipients[0].status, read_at: i.recipients[0].read_at } : {});
        return !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
      }).length
      : 0;
    const total = Array.isArray(items) ? items.length : 0;
    const peers = await agtalkClient.listParticipants();
    const peersOnline = Array.isArray(peers)
      ? peers.filter((p) => p.status === 'online').length
      : 0;
    return { unread, total, peersOnline };
  } catch (err) {
    console.error('[BG] inbox 统计失败:', err.message);
    return fallback;
  }
}

async function agtalkMarkDone(msgId, agent) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const target = agent || runtimeConfig.agentName;
  const data = await agtalkClient.done(msgId, target);
  return { ok: true, data };
}

async function agtalkMarkRead(msgId, agent) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const target = agent || runtimeConfig.agentName;
  try {
    const data = await agtalkClient.read(msgId, target);
    return { ok: true, data };
  } catch (err) {
    return { ok: false, error: err.message };
  }
}

async function agtalkGetPeers() {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const participants = await agtalkClient.listParticipants();
  return { ok: true, peers: participants || [] };
}

async function getConnectedPeerDetails(tabId = null) {
  const connected = new Set(getConnectedPeers());
  let peers = [];
  try {
    const result = await agtalkGetPeers();
    peers = Array.isArray(result.peers)
      ? result.peers
          .filter((peer) => peer.name && peer.name !== runtimeConfig.agentName)
          .map((peer) => ({
            ...peer,
            connected: connected.has(peer.name),
            active: peer.name === runtimeConfig.activePeer,
          }))
      : [];
  } catch (err) {
    console.warn('[BG] 获取 peer 列表失败，降级使用本地连接配置:', err.message);
    peers = getConnectedPeers().map((name) => ({
      name,
      type: 'peer',
      role: 'connected',
      status: 'unknown',
      connected: true,
      active: name === runtimeConfig.activePeer,
    }));
  }
  const known = new Set(peers.map((peer) => peer.name));
  for (const name of getConnectedPeers()) {
    if (!known.has(name)) {
      peers.push({
        name, type: 'peer', role: 'connected', status: 'unknown',
        connected: true, active: name === runtimeConfig.activePeer,
      });
    }
  }
  peers.sort((a, b) => {
    const aScore = (a.connected ? 0 : 1) + (a.active ? -1 : 0);
    const bScore = (b.connected ? 0 : 1) + (b.active ? -1 : 0);
    if (aScore !== bScore) return aScore - bScore;
    return a.name.localeCompare(b.name);
  });
  return {
    ok: true, peers,
    activePeer: runtimeConfig.activePeer || '',
    connectedPeers: getConnectedPeers(),
    recommendedPeer: getRecommendedPeerForTab(tabId),
  };
}

async function agtalkDetail(msgId, agent) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const target = agent || runtimeConfig.agentName;
  const data = await agtalkClient.detail(msgId, target);
  return { ok: true, item: data };
}

async function agtalkAttachment(attachmentId, agent) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const target = agent || runtimeConfig.agentName;
  const data = await agtalkClient.attachment(attachmentId, target);
  return { ok: true, attachment: data?.attachment, content: data?.content };
}

// Service Worker 消息总线
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  const keepAlive = setInterval(() => {}, 1000);
  const finish = (value) => {
    clearInterval(keepAlive);
    sendResponse(value);
  };

  (async () => {
    try {
      await initPromise;

      switch (message.type) {
      case 'CHAT_TURN': {
      if (runtimeConfig.enabled && runtimeConfig.autoForward) {
        forwardToAgtalk(message.payload)
          .then(() => finish({ ok: true }))
          .catch((err) => finish({ ok: false, error: err.message }));
      } else {
        finish({ ok: true, skipped: true });
      }
      return true;
    }
    case 'GET_CONFIG':
      finish({ config: runtimeConfig });
      return true;
    case 'SAVE_CONFIG': {
      const urlChanged = message.config.daemonUrl && message.config.daemonUrl !== getDaemonUrl();
      runtimeConfig = { ...runtimeConfig, ...message.config };
      chrome.storage.local.set({ agtalk_config: runtimeConfig }, () => {
        if (urlChanged) {
          agtalkClient = new AgtalkClient(getDaemonUrl());
          console.log('[BG] daemon URL 已更新:', getDaemonUrl());
        }
        reconcileInboxPolling();
        finish({ ok: true });
      });
      return true;
    }
    case 'REGISTER_AGENT':
      ensureJoined().then(() => finish({ ok: !!agtalkClient?.sessionId, session_id: agtalkClient?.sessionId })).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'RECONNECT':
      connectionState.reconnecting = true;
      ensureJoined().then(async () => {
        const connected = await agtalkClient?.ping();
        if (connected) {
          connectionState = { connected: true, error: null, reconnecting: false };
          reconnectAttempt = 0;
          finish({ ok: true });
        } else {
          finish({ ok: false, error: '重连后 ping 失败' });
        }
      }).catch((err) => {
        connectionState = { connected: false, error: err.message, reconnecting: false };
        finish({ ok: false, error: err.message });
      });
      return true;
    case 'GET_RECENT_MESSAGES':
      MessageStore.getRecent(message.limit || 100).then((items) => finish({ ok: true, items })).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'SEARCH_MESSAGES':
      MessageStore.search(message.query || '', message.limit || 50).then((items) => finish({ ok: true, items })).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_SEND':
      agtalkDirectSend(message.toAgent, message.body, message.subject, message.replyTo).then(async (result) => {
        if (result?.ok && sender?.tab?.id && message.toAgent) {
          await associateTabPeer(sender.tab.id, message.toAgent, sender.tab.url || '');
        }
        finish(result);
      }).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_INBOX':
      agtalkInbox(message.agent, message.status).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_INBOX_STATS':
      agtalkInboxStats().then((stats) => finish({ ok: true, ...stats })).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_MARK_DONE':
      agtalkMarkDone(message.msgId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_MARK_READ':
      agtalkMarkRead(message.msgId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'CHECK_AGTALK_STATUS':
      checkAgtalkConnection().then(finish).catch((err) => finish({ connected: false, error: err.message }));
      return true;
    case 'AGTALK_PEERS':
      agtalkGetPeers().then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'DELIVER_TO_ACTIVE_TAB': {
      const tabs = await new Promise((resolve) => chrome.tabs.query({ active: true, currentWindow: true }, resolve));
      if (!Array.isArray(tabs) || tabs.length === 0) {
        finish({ ok: false, error: '未找到当前标签页' });
        return true;
      }
      const tab = tabs[0];
      try {
        await ensureContentScriptInjected(tab.id);
        const res = await new Promise((resolve, reject) => {
          chrome.tabs.sendMessage(tab.id, { type: 'AGTALK_INCOMING', item: message.item }, (r) => {
            if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
            resolve(r);
          });
        });
        // 注入成功后存储消息、标记已注入、自动关联 tab
        const fromPeer = message.item?.from?.name || message.item?.from_agent || '';
        if (fromPeer) {
          await associateTabPeer(tab.id, fromPeer, tab.url || '');
        }
        finish({ ok: true, result: res });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
    case 'ASSOCIATE_TAB_PEER': {
      const tabId = sender?.tab?.id || message.tabId;
      const url = sender?.tab?.url || message.url || '';
      if (tabId == null) {
        finish({ ok: false, error: '缺少 tabId（只能在 content script 中调用）' });
        return true;
      }
      try {
        await associateTabPeer(tabId, message.peer || '', url);
        finish({ ok: true, peer: message.peer || '', tabId });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
    case 'GET_TAB_ASSOCIATION': {
      const tabId = sender?.tab?.id || message.tabId;
      const linkedPeer = getRecommendedPeerForTab(tabId);
      finish({ ok: true, peer: linkedPeer, tabId });
      return true;
    }
    case 'SET_AUTO_REPLY_PEER': {
      const tabId = sender?.tab?.id || message.tabId;
      const url = sender?.tab?.url || message.url || '';
      if (tabId == null) {
        finish({ ok: false, error: '缺少 tabId（只能在 content script 中调用）' });
        return true;
      }
      try {
        await setAutoReplyPeer(tabId, message.peer || '', url);
        finish({ ok: true, peer: message.peer || '', tabId });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
    case 'GET_AUTO_REPLY_PEER': {
      const tabId = sender?.tab?.id || message.tabId;
      const peer = getAutoReplyPeer(tabId);
      finish({ ok: true, peer, tabId });
      return true;
    }
    case 'PAUSE_AUTO_REPLY': {
      const tabId = sender?.tab?.id || message.tabId;
      const url = sender?.tab?.url || message.url || '';
      if (tabId != null) {
        await setAutoReplyPeer(tabId, '', url);
      }
      finish({ ok: true, peer: '', tabId });
      return true;
    }
    case 'PAUSE_ALL_AUTO_REPLY': {
      try {
        // 清除所有 tab 的 autoReplyPeer
        const keys = Object.keys(tabPeerHints);
        for (const tabIdStr of keys) {
          const hint = tabPeerHints[tabIdStr];
          if (hint?.autoReplyPeer) {
            await setAutoReplyPeer(Number(tabIdStr), '');
          }
        }
        // 通知所有匹配标签页停止自动回复
        const patterns = ['https://chatgpt.com/*', 'https://claude.ai/*', 'https://sider.ai/*', 'https://chatglm.cn/*'];
        const allTabs = await new Promise((resolve) => chrome.tabs.query({ url: patterns }, resolve));
        if (Array.isArray(allTabs)) {
          for (const tab of allTabs) {
            chrome.tabs.sendMessage(tab.id, { type: 'PAUSE_AUTO_REPLY' }, () => {
              chrome.runtime.lastError;
            });
          }
        }
        finish({ ok: true });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
    case 'GET_CONNECTED_PEERS':
      getConnectedPeerDetails(sender?.tab?.id || null).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'OPEN_INBOX':
      chrome.tabs.create({ url: chrome.runtime.getURL('inbox/inbox.html') }, () => {
        if (chrome.runtime.lastError) {
          finish({ ok: false, error: chrome.runtime.lastError.message });
        } else {
          finish({ ok: true });
        }
      });
      return true;
    case 'AGTALK_DETAIL':
      agtalkDetail(message.msgId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_ATTACHMENT':
      agtalkAttachment(message.attachmentId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    default:
      finish({ ok: false, error: '未知消息类型: ' + message.type });
      return true;
    }
    } catch (err) {
      console.error('[BG] 消息处理异常:', err);
      finish({ ok: false, error: err.message });
    }
  })();

  return true;
});
