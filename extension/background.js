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
  activePeer: '',
  connectedPeers: [],
  enabled: true,
  autoForward: false,
  autoReceive: true,
  autoInject: false,
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
let runtimeConfig = normalizeConfig(DEFAULT_CONFIG);
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
chrome.storage.local.get(['agtalk_config', 'agtalk_session', 'agtalk_tab_peer_hints'], (result) => {
  if (result.agtalk_config) {
    const normalized = normalizeConfig(result.agtalk_config);
    runtimeConfig = normalized;
    if (JSON.stringify(normalized) !== JSON.stringify(result.agtalk_config)) {
      chrome.storage.local.set({ agtalk_config: normalized });
    }
  }
  tabPeerHints = normalizeTabPeerHints(result.agtalk_tab_peer_hints);
  initAgtalkClient(result.agtalk_session).then(() => {
    initResolve();
  }).catch((err) => {
    console.error('[BG] 初始化失败:', err.message);
    initResolve();
  });
});

chrome.storage.onChanged.addListener((changes) => {
  if (changes.agtalk_config) {
    runtimeConfig = normalizeConfig(changes.agtalk_config.newValue);
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

function normalizeConfig(config) {
  const merged = { ...DEFAULT_CONFIG, ...(config || {}) };
  const connectedPeers = Array.isArray(merged.connectedPeers)
    ? merged.connectedPeers.map((peer) => String(peer).trim()).filter(Boolean)
    : [];
  const legacyTarget = merged.targetAgent ? String(merged.targetAgent).trim() : '';
  if (legacyTarget && !connectedPeers.includes(legacyTarget)) {
    connectedPeers.unshift(legacyTarget);
  }
  merged.connectedPeers = Array.from(new Set(connectedPeers));
  const activePeer = merged.activePeer ? String(merged.activePeer).trim() : '';
  if (activePeer && !merged.connectedPeers.includes(activePeer)) {
    merged.connectedPeers.unshift(activePeer);
  }
  merged.activePeer = merged.connectedPeers.includes(activePeer)
    ? activePeer
    : (merged.connectedPeers[0] || legacyTarget || '');
  merged.targetAgent = merged.activePeer || legacyTarget || '';
  return merged;
}

async function persistRuntimeConfig() {
  return new Promise((resolve) => {
    chrome.storage.local.set({ agtalk_config: runtimeConfig }, resolve);
  });
}

function normalizeTabPeerHints(hints) {
  if (!hints || typeof hints !== 'object' || Array.isArray(hints)) return {};
  const result = {};
  for (const [tabId, value] of Object.entries(hints)) {
    if (!value || typeof value !== 'object') continue;
    const peer = String(value.peer || '').trim();
    if (!peer) continue;
    result[String(tabId)] = {
      peer,
      url: String(value.url || ''),
      updated_at: Number(value.updated_at || Date.now()),
    };
  }
  return result;
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
  if (!peer) {
    if (tabPeerHints[key]) {
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
      peer: String(peer).trim(),
      url: String(url || ''),
      updated_at: Date.now(),
    },
  };
  await persistTabPeerHints();
}

function isConnectedPeer(name) {
  return !!name && getConnectedPeers().includes(name);
}

function resolveTargetPeer(explicit) {
  if (explicit) {
    return isConnectedPeer(explicit) ? explicit : '';
  }
  const target = runtimeConfig.activePeer || runtimeConfig.targetAgent || '';
  if (target && isConnectedPeer(target)) return target;
  return getConnectedPeers()[0] || '';
}

async function hydrateLocalFlags(items) {
  if (!Array.isArray(items) || items.length === 0) return items || [];
  const hydrated = await Promise.all(items.map(async (item) => {
    try {
      const local = await MessageStore.getById(item.id);
      if (!local) return item;
      return { ...item, _injected: !!local.injected, injected: !!local.injected };
    } catch (err) {
      return item;
    }
  }));
  return hydrated;
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
      if (runtimeConfig.autoInject) {
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

  // 1. 若消息来源 peer 有关联 tab，优先注入到该 tab
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
        return;
      } catch (err) {
        console.warn('[BG] 注入已关联 tab 失败，回退到当前激活标签页:', err.message);
      }
    }
  }

  // 2. 优先注入到当前激活的匹配标签页，避免同时注入到多个后台标签页
  const activeTabs = await new Promise((resolve) => chrome.tabs.query({ active: true, currentWindow: true, url: patterns }, resolve));
  if (Array.isArray(activeTabs) && activeTabs.length > 0) {
    try {
      await ensureContentScriptInjected(activeTabs[0].id);
      await new Promise((resolve, reject) => {
        chrome.tabs.sendMessage(activeTabs[0].id, message, (res) => {
          if (chrome.runtime.lastError) return reject(new Error(chrome.runtime.lastError.message));
          resolve(res);
        });
      });
      notifyAutoInject(item);
    } catch (err) {
      console.error('[BG] 自动注入当前标签页失败:', err.message);
    }
    return;
  }

  // 3. 没有激活的匹配标签页时，回退到所有匹配标签页（仅发送，不主动注入后台标签页）
  for (const tab of allTabs) {
    chrome.tabs.sendMessage(tab.id, message, () => {
      if (chrome.runtime.lastError) {
        console.error('[BG] 发送 AGTALK_INCOMING 到后台标签页失败:', tab.id, chrome.runtime.lastError.message);
      }
    });
  }
  notifyAutoInject(item);
}

function findTabIdByPeer(peer, tabs) {
  if (!peer || !tabs) return null;
  for (const [tabId, hint] of Object.entries(tabPeerHints)) {
    if (hint.peer === peer) {
      const id = Number(tabId);
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
    console.log('[BG] 未指定目标 Agent，跳过转发');
    return;
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
  // daemon 返回 { message: {...}, notify: {...} }
  const msg = data?.message;
  const msgId = msg?.id || data?.id || `tmp-${Date.now()}`;
  console.log(`[BG] agtalk 转发成功: target=${toAgent}, msg_id=${msgId}`);
  const saveItem = msg && msg.id ? msg : {
    id: msgId,
    chat_id: payload.conversation_id,
    from: { name: runtimeConfig.agentName, type: 'web' },
    recipients: [{ recipient_name: toAgent, status: 'pending' }],
    subject: payload.conversation_id,
    content: { body: payload.turn.assistant },
    content_type: 'text',
    created_at: new Date().toISOString(),
    reply_to_id: payload.replyToMsgId,
  };
  MessageStore.save(saveItem).catch((err) => console.error('[BG] 保存转发消息失败:', err.message, 'item.id=', saveItem.id));
  return data;
}

async function agtalkDirectSend(toAgent, body, subject, replyTo) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };
  const target = resolveTargetPeer(toAgent);
  if (!target) return { ok: false, error: '未指定目标 Agent' };

  const data = await agtalkClient.send({
    to: target,
    body: body || '',
    conversationId: subject || null,
    replyTo: replyTo || null,
    contentType: 'text',
    notify: true,
  });
  // daemon 返回 { message: {...}, notify: {...} }
  const msg = data?.message;
  const msgId = msg?.id || data?.id || `tmp-${Date.now()}`;
  const saveItem = msg && msg.id ? msg : {
    id: msgId,
    from: { name: runtimeConfig.agentName, type: 'web' },
    recipients: [{ recipient_name: target, status: 'pending' }],
    subject,
    content: { body },
    content_type: 'text',
    created_at: new Date().toISOString(),
    reply_to_id: replyTo,
  };
  MessageStore.save(saveItem).catch((err) => console.error('[BG] 保存发送消息失败:', err.message, 'item.id=', saveItem.id));
  return { ok: true, msg_id: msgId, to: target, data };
}

async function agtalkInbox(agent, status) {
  if (!agtalkClient?.sessionId) await ensureJoined();
  if (!agtalkClient?.sessionId) return { ok: false, error: 'agtalk 未连接' };

  const target = agent || runtimeConfig.agentName;
  const items = await agtalkClient.inbox({ participant: target, status: status || 'all', limit: 1000, peek: false });
  return { ok: true, agent: target, count: items.length, items: await hydrateLocalFlags(items) };
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
        name,
        type: 'peer',
        role: 'connected',
        status: 'unknown',
        connected: true,
        active: name === runtimeConfig.activePeer,
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
    ok: true,
    peers,
    activePeer: runtimeConfig.activePeer || '',
    connectedPeers: getConnectedPeers(),
    recommendedPeer: getRecommendedPeerForTab(tabId),
  };
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
      runtimeConfig = normalizeConfig({ ...runtimeConfig, ...message.config });
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
      MessageStore.getRecent(message.limit || 100).then((items) => hydrateLocalFlags(items)).then((items) => finish({ ok: true, items })).catch((err) => finish({ ok: false, error: err.message }));
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
    case 'GET_CONNECTED_PEERS':
      getConnectedPeerDetails(sender?.tab?.id || null).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'GET_TAB_ASSOCIATION': {
      const tabId2 = sender?.tab?.id || message.tabId;
      const linkedPeer = tabId2 != null ? getRecommendedPeerForTab(tabId2) : '';
      finish({ ok: true, peer: linkedPeer, tabId: tabId2 });
      return true;
    }
    case 'ASSOCIATE_TAB_PEER': {
      const tabId = sender?.tab?.id || message.tabId;
      const url = sender?.tab?.url || message.url || '';
      if (tabId == null || !message.peer) {
        finish({ ok: false, error: '缺少 tabId 或 peer' });
        return true;
      }
      try {
        await associateTabPeer(tabId, message.peer, url);
        finish({ ok: true, peer: message.peer, tabId });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
    case 'OPEN_INBOX':
      chrome.tabs.create({ url: chrome.runtime.getURL('inbox/inbox.html') }, () => {
        if (chrome.runtime.lastError) {
          finish({ ok: false, error: chrome.runtime.lastError.message });
        } else {
          finish({ ok: true });
        }
      });
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
      checkAgtalkConnection().then((status) => finish({ ...status, activePeer: runtimeConfig.activePeer || '', connectedPeers: getConnectedPeers() })).catch((err) => finish({ connected: false, error: err.message }));
      return true;
    case 'AGTALK_PEERS':
      agtalkGetPeers().then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_DETAIL':
      agtalkDetail(message.msgId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
      return true;
    case 'AGTALK_ATTACHMENT':
      agtalkAttachment(message.attachmentId, message.agent).then(finish).catch((err) => finish({ ok: false, error: err.message }));
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
        if (res?.ok) {
          await MessageStore.save(message.item).catch(() => {});
          await MessageStore.markInjected(message.item.id).catch(() => {});
          await associateTabPeer(tab.id, message.item.from?.name || message.item.from_agent || message.item.recipients?.[0]?.recipient_name || '', tab.url || '');
        }
        finish({ ok: true, result: res });
      } catch (err) {
        finish({ ok: false, error: err.message });
      }
      return true;
    }
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
