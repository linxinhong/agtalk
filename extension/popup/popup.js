// Popup UI — agtalk Web Bridge 主界面（精简版）
const $ = (id) => document.getElementById(id);

// 默认配置：打开即用
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

let currentConfig = { ...DEFAULT_CONFIG };
let peers = [];
let inboxItems = [];
let inboxPeerFilter = 'all';
let inboxPeerFilterInitialized = false;
let connectedPeerItems = [];
let availablePeerItems = [];
let agentSearchQuery = '';

async function load() {
  await ensureConfig();
  renderAutoInjectButton();
  if (currentConfig.enabled && currentConfig.agentName) {
    await registerAgent();
  }
  await refreshStatus();
  await loadPeers();
  await loadLocalInbox(); // 先显示本地缓存，实现秒开
  await loadInbox();      // 再从服务器刷新
}

async function loadLocalInbox() {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'GET_RECENT_MESSAGES', limit: 50 }, resolve);
  });
  if (!result?.ok || !Array.isArray(result.items) || result.items.length === 0) return;
  inboxItems = result.items.map((msg) => ({
    id: msg.id,
    chat_id: msg.chat_id,
    from: { name: msg.from_name, type: msg.from_type },
    content: { body: msg.body },
    subject: msg.subject,
    created_at: msg.created_at,
    delivery: { status: msg.status, read_at: msg.read_at, done_at: msg.done_at },
    attachments: msg.attachments || [],
    _injected: !!msg.injected,
    _local: true,
  }));
  renderInboxList('<li class="empty">本地缓存</li>');
}

async function ensureConfig() {
  const result = await chrome.storage.local.get(['agtalk_config']);
  if (result.agtalk_config) {
    currentConfig = normalizeConfig(result.agtalk_config);
  } else {
    currentConfig = normalizeConfig(DEFAULT_CONFIG);
    await saveConfig();
  }
}

async function saveConfig() {
  currentConfig = normalizeConfig(currentConfig);
  await chrome.storage.local.set({ agtalk_config: currentConfig });
  return new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'SAVE_CONFIG', config: currentConfig }, resolve);
  });
}

function normalizeConfig(cfg) {
  const merged = { ...DEFAULT_CONFIG, ...(cfg || {}) };
  const connectedPeers = Array.isArray(merged.connectedPeers)
    ? merged.connectedPeers.map((peer) => String(peer).trim()).filter(Boolean)
    : [];
  const legacyTarget = merged.targetAgent ? String(merged.targetAgent).trim() : '';
  if (legacyTarget && !connectedPeers.includes(legacyTarget)) {
    connectedPeers.unshift(legacyTarget);
  }
  merged.connectedPeers = Array.from(new Set(connectedPeers));
  const activePeer = merged.activePeer ? String(merged.activePeer).trim() : '';
  merged.activePeer = activePeer && merged.connectedPeers.includes(activePeer)
    ? activePeer
    : (merged.connectedPeers[0] || legacyTarget || '');
  merged.targetAgent = merged.activePeer || legacyTarget || '';
  return merged;
}

async function registerAgent() {
  if (!currentConfig.agentName) return;
  return new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'REGISTER_AGENT' }, resolve);
  });
}

function renderAutoInjectButton() {
  const btn = $('auto-inject-btn');
  if (!btn) return;
  if (currentConfig.autoInject) {
    btn.className = 'icon-btn connected';
    btn.title = '自动注入已开启（点击关闭）';
  } else {
    btn.className = 'icon-btn disconnected';
    btn.title = '点击开启自动注入';
  }
}

async function toggleAutoInject() {
  currentConfig.autoInject = !currentConfig.autoInject;
  await saveConfig();
  renderAutoInjectButton();
}

async function refreshStatus() {
  const status = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'CHECK_AGTALK_STATUS' }, resolve);
  });

  const dot = $('status-dot');
  const agentEl = $('agent-status');
  const inboxEl = $('inbox-status');
  const peersEl = $('peers-status');
  const detailEl = $('connection-detail');
  const reconnectBtn = $('reconnect-btn');

  if (status?.connected) {
    dot.className = 'dot online';
    dot.title = status.url || 'daemon 在线';
    agentEl.textContent = status.agent || currentConfig.agentName || '已连接';
    agentEl.className = 'online';
    const unread = status.inboxUnread || 0;
    const total = status.inboxTotal || 0;
    inboxEl.textContent = unread > 0 ? `未读 ${unread}/${total}` : `${total} 条消息`;
    inboxEl.className = unread > 0 ? 'error' : 'muted';
    peersEl.textContent = `${status.peersOnline || 0} peers`;
    peersEl.className = 'muted';
    detailEl.classList.add('hidden');
    reconnectBtn.classList.add('hidden');
  } else if (status?.reconnecting) {
    dot.className = 'dot reconnecting';
    dot.title = '正在重连...';
    agentEl.textContent = '连接中...';
    agentEl.className = 'warn';
    inboxEl.textContent = '-';
    inboxEl.className = 'muted';
    peersEl.textContent = '-';
    peersEl.className = 'muted';
    detailEl.textContent = status?.error || '正在尝试重连 daemon';
    detailEl.classList.remove('hidden');
    reconnectBtn.classList.add('hidden');
  } else {
    dot.className = 'dot offline';
    dot.title = status?.error || 'daemon 离线';
    agentEl.textContent = '未连接';
    agentEl.className = 'error';
    inboxEl.textContent = '-';
    inboxEl.className = 'muted';
    peersEl.textContent = '-';
    peersEl.className = 'muted';
    detailEl.textContent = status?.error ? `错误: ${status.error}` : 'daemon 离线';
    detailEl.classList.remove('hidden');
    reconnectBtn.classList.remove('hidden');
  }
}

async function reconnectDaemon() {
  const btn = $('reconnect-btn');
  btn.disabled = true;
  btn.textContent = '重连中...';
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'RECONNECT' }, resolve);
  });
  btn.disabled = false;
  btn.textContent = '重连';
  await refreshStatus();
  if (result?.ok) {
    alert('重连成功');
  } else {
    alert('重连失败: ' + (result?.error || '未知错误'));
  }
}

async function loadInbox() {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'AGTALK_INBOX', status: 'all' }, resolve);
  });
  if (!result?.ok || !Array.isArray(result.items)) {
    if (inboxItems.length === 0) {
      $('inbox-list').innerHTML = '<li class="empty">加载失败</li>';
    }
    return;
  }
  inboxItems = result.items;
  renderInboxList();
}

function renderAttachments(attachments) {
  if (!attachments || attachments.length === 0) return '';
  const tags = attachments.map((att) =>
    `<button class="attachment-btn" data-id="${att.id}" data-filename="${escapeHtml(att.filename)}">📎 ${escapeHtml(att.filename)}</button>`
  ).join('');
  return `<div class="attachment-list">${tags}</div>`;
}

function renderInboxList(emptyHint) {
  const list = $('inbox-list');
  const filteredItems = filterInboxByPeer(inboxItems, inboxPeerFilter);
  if (filteredItems.length === 0) {
    list.innerHTML = emptyHint || '<li class="empty">暂无消息</li>';
    return;
  }

  list.innerHTML = filteredItems.map((item) => {
    const delivery = item.delivery || (item.recipients?.[0] ? { status: item.recipients[0].status, read_at: item.recipients[0].read_at } : {});
    const isUnread = !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
    const body = item.content?.body || item.body || '';
    const shortBody = escapeHtml(body).slice(0, 80);
    const localTag = item._local ? '<span class="local-tag">本地</span> ' : '';
    const injectedTag = item._injected || item.injected ? '<span class="injected-tag">已注入</span> ' : '';
    const msgId = item.id ? item.id.slice(0, 8) : '-';
    return `
      <li class="inbox-item ${isUnread ? 'unread' : ''}" data-id="${item.id}">
        <div class="inbox-meta">
          <span class="from">${localTag}${item.from?.name || '未知'}</span>
          <span class="time">${formatTime(item.created_at)}</span>
        </div>
        <div class="inbox-submeta">${injectedTag}<span class="msg-id">msg-id: ${msgId}</span></div>
        <div class="preview">${shortBody}</div>
        <div class="detail-body">${escapeHtml(body)}${renderAttachments(item.attachments)}</div>
        <div class="inbox-actions">
          <button class="inject-btn" data-id="${item.id}">注入对话框</button>
          <button class="reply-btn" data-id="${item.id}">回复</button>
        </div>
      </li>
    `;
  }).join('');

  bindInboxEvents();
}

function bindInboxEvents() {
  const list = $('inbox-list');
  list.querySelectorAll('.inbox-item').forEach((itemEl) => {
    itemEl.addEventListener('click', (e) => {
      if (e.target.tagName === 'BUTTON') return;
      document.querySelectorAll('.inbox-item').forEach((el) => {
        if (el !== itemEl) el.classList.remove('expanded');
      });
      itemEl.classList.toggle('expanded');
    });
  });
  list.querySelectorAll('.inject-btn').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      injectMessage(btn.dataset.id);
    });
  });
  list.querySelectorAll('.reply-btn').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      replyToMessage(btn.dataset.id);
    });
  });
  list.querySelectorAll('.attachment-btn').forEach((btn) => {
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const content = await fetchAttachmentContent(btn.dataset.id);
      if (content !== null) {
        alert(`附件: ${btn.dataset.filename}\n\n${content.slice(0, 2000)}${content.length > 2000 ? '\n\n(已截断显示前 2000 字符)' : ''}`);
      }
    });
  });
}

async function fetchAttachmentContent(attachmentId) {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'AGTALK_ATTACHMENT', attachmentId }, resolve);
  });
  if (!result?.ok) {
    console.warn('[Popup] 附件获取失败:', result?.error);
    return null;
  }
  return result.content;
}

async function buildInjectableBody(item) {
  const attachments = item.attachments || [];
  let mainBody = item.content?.body || item.body || '';
  const extraParts = [];
  for (const att of attachments) {
    if (att.role === 'full_body' || att.filename?.includes('full_body')) {
      const full = await fetchAttachmentContent(att.id);
      if (full !== null) mainBody = full;
    } else if (att.content_type?.startsWith('text/')) {
      const text = await fetchAttachmentContent(att.id);
      if (text !== null) extraParts.push(`--- 附件: ${att.filename} ---\n${text}`);
    } else {
      extraParts.push(`--- 附件: ${att.filename} (${att.size} bytes, ${att.content_type}) ---`);
    }
  }
  return [mainBody, ...extraParts].filter(Boolean).join('\n\n');
}

async function injectMessage(msgId) {
  let item = inboxItems.find((i) => i.id === msgId);
  if (!item) return;

  // 注入前拉取完整消息，避免使用 inbox 列表里的截断/预览内容
  const detail = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'AGTALK_DETAIL', msgId }, resolve);
  });
  if (detail?.ok && detail.item) {
    item = normalizeMessageItem(detail.item);
    // 同时更新本地缓存，下次列表渲染时显示完整内容
    const idx = inboxItems.findIndex((i) => i.id === item.id);
    if (idx >= 0) inboxItems[idx] = item;
  } else if (detail?.error) {
    console.warn('[Popup] detail 拉取失败，将使用本地缓存（可能截断）:', detail.error);
  }

  // 拉取附件并把完整内容拼进 body
  const fullBody = await buildInjectableBody(item);
  item = { ...item, body: fullBody, content: { body: fullBody } };

  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'DELIVER_TO_ACTIVE_TAB', item }, resolve);
  });
  if (result?.ok) {
    item._injected = true;
    const idx = inboxItems.findIndex((i) => i.id === item.id);
    if (idx >= 0) inboxItems[idx] = item;
    renderInboxList();
    console.log('[Popup] 已注入消息:', msgId);
  } else {
    alert('注入失败: ' + (result?.error || '未知错误'));
  }
}

async function replyToMessage(msgId) {
  const item = inboxItems.find((i) => i.id === msgId);
  if (!item) return;
  const to = item.from?.name || item.from_agent;
  if (!to) return;
  const body = prompt('回复 ' + to + ':');
  if (!body) return;
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({
      type: 'AGTALK_SEND',
      toAgent: to,
      body,
      replyTo: item.id,
    }, resolve);
  });
  if (result?.ok) {
    await loadInbox();
  } else {
    alert('回复失败: ' + (result?.error || '未知错误'));
  }
}

async function loadPeers() {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'GET_CONNECTED_PEERS' }, resolve);
  });
  if (!result?.ok || !Array.isArray(result.peers)) {
    if (currentConfig.connectedPeers?.length) {
      peers = currentConfig.connectedPeers.map((name) => ({
        name,
        type: 'peer',
        role: 'connected',
        status: 'unknown',
        connected: true,
        active: name === currentConfig.activePeer,
      }));
      connectedPeerItems = peers;
      availablePeerItems = [];
      renderAgentLists();
      renderInboxPeerTabs(peers);
      renderPeerSummary(peers);
      return;
    }
    peers = [];
    renderPeerSummary();
    renderConnectedPeers([]);
    renderAvailablePeers([]);
    renderInboxPeerTabs([]);
    return;
  }
  peers = result.peers.filter((p) => p.name !== currentConfig.agentName);
  const connectedNames = new Set(result.connectedPeers || currentConfig.connectedPeers || []);
  currentConfig.connectedPeers = Array.from(connectedNames);
  currentConfig.activePeer = result.activePeer || currentConfig.activePeer || currentConfig.connectedPeers[0] || '';
  currentConfig.targetAgent = currentConfig.activePeer;
  if (!inboxPeerFilterInitialized) {
    inboxPeerFilter = currentConfig.activePeer || 'all';
    inboxPeerFilterInitialized = true;
  }
  if (inboxPeerFilter !== 'all' && inboxPeerFilter !== '__unconnected__' && !connectedNames.has(inboxPeerFilter)) {
    inboxPeerFilter = 'all';
  }
  const peerByName = new Map(peers.map((p) => [p.name, p]));
  const connected = currentConfig.connectedPeers.map((name) =>
    peerByName.get(name) || { name, type: 'peer', role: 'connected', status: 'not listed' }
  );
  connectedPeerItems = connected;
  availablePeerItems = peers.filter((p) => !connectedNames.has(p.name));
  renderAgentLists();
  renderInboxPeerTabs(connected);
  renderPeerSummary(connected);
}

function renderAgentLists() {
  renderConnectedPeers(filterPeersBySearch(connectedPeerItems));
  renderAvailablePeers(filterPeersBySearch(availablePeerItems));
}

function filterPeersBySearch(items) {
  const query = agentSearchQuery.trim().toLowerCase();
  if (!query) return items;
  return items.filter((peer) => {
    const haystack = [
      peer.name,
      peer.type,
      peer.role,
      peer.status,
      peer.transport,
    ].filter(Boolean).join(' ').toLowerCase();
    return haystack.includes(query);
  });
}

function renderConnectedPeers(connectedPeers) {
  const container = $('connected-peer-list');
  const agentsActiveLabel = $('agents-active-label');
  if (!container || !agentsActiveLabel) return;
  if (!connectedPeers.length) {
    container.innerHTML = `<div class="empty-inline">${agentSearchQuery ? '没有匹配的已连接 Agent' : '尚未连接任何 Peer'}</div>`;
    agentsActiveLabel.textContent = '无 active peer';
    return;
  }
  container.innerHTML = connectedPeers.map((p) => {
    const active = p.name === currentConfig.activePeer;
    const status = p.status ? ` · ${p.status}` : '';
    return `
      <div class="peer-row ${active ? 'active' : ''}" data-name="${p.name}">
        <div class="peer-main">
          <span class="peer-name">${escapeHtml(p.name)}</span>
          <span class="peer-role">${escapeHtml(peerDescription({ ...p, status: p.status || status.replace(' · ', '') }))}</span>
        </div>
        <div class="peer-actions">
          <button class="peer-btn set-active" data-name="${p.name}">${active ? '当前' : '设为当前'}</button>
          <button class="peer-btn disconnect" data-name="${p.name}">移除</button>
        </div>
      </div>
    `;
  }).join('');
  container.querySelectorAll('.set-active').forEach((btn) => {
    btn.addEventListener('click', () => setActivePeer(btn.dataset.name));
  });
  container.querySelectorAll('.disconnect').forEach((btn) => {
    btn.addEventListener('click', () => disconnectPeer(btn.dataset.name));
  });
}

function renderPeerSummary(connected = []) {
  const activeLabel = $('active-peer-label');
  const agentsActiveLabel = $('agents-active-label');
  const manageBtn = $('manage-peers-btn');
  const count = currentConfig.connectedPeers?.length || connected.length || 0;
  const activeText = currentConfig.activePeer || '无 active peer';
  if (activeLabel) {
    activeLabel.textContent = count > 0 ? `${activeText} · 已连接 ${count}` : activeText;
  }
  if (manageBtn) {
    manageBtn.title = count > 0 ? `Agent 管理: ${activeText} · 已连接 ${count}` : 'Agent 管理';
  }
  if (agentsActiveLabel) {
    agentsActiveLabel.textContent = count > 0 ? `当前: ${activeText} · 已连接 ${count}` : '无 active peer';
  }
}

function renderAvailablePeers(availablePeers) {
  const container = $('available-peer-list');
  if (!container) return;
  if (!availablePeers.length) {
    container.innerHTML = `<div class="empty-inline">${agentSearchQuery ? '没有匹配的可连接 Agent' : '没有可连接的 Peer'}</div>`;
    return;
  }
  container.innerHTML = availablePeers.map((p) => `
    <div class="peer-row" data-name="${p.name}">
      <div class="peer-main">
        <span class="peer-name">${escapeHtml(p.name)}</span>
        <span class="peer-role">${escapeHtml(peerDescription(p))}</span>
      </div>
      <div class="peer-actions">
        <button class="peer-btn connect-peer" data-name="${p.name}">连接</button>
      </div>
    </div>
  `).join('');
  container.querySelectorAll('.connect-peer').forEach((btn) => {
    btn.addEventListener('click', () => connectPeer(btn.dataset.name));
  });
}

function renderInboxPeerTabs(connectedPeers) {
  const tabs = $('peer-tabs');
  if (!tabs) return;
  const items = [
    { name: 'all', label: 'All' },
    ...connectedPeers.map((p) => ({ name: p.name, label: p.name })),
    { name: '__unconnected__', label: '未连接' },
  ];
  tabs.innerHTML = items.map((item) => `
    <button class="peer-tab ${item.name === inboxPeerFilter ? 'active' : ''}" data-peer="${item.name}">${escapeHtml(item.label)}</button>
  `).join('');
  tabs.querySelectorAll('.peer-tab').forEach((btn) => {
    btn.addEventListener('click', () => {
      inboxPeerFilter = btn.dataset.peer || 'all';
      renderInboxList();
      renderInboxPeerTabs(connectedPeers);
    });
  });
}

function filterInboxByPeer(items, peerName) {
  if (!peerName || peerName === 'all') return items;
  if (peerName === '__unconnected__') {
    const connected = new Set(currentConfig.connectedPeers || []);
    return items.filter((item) => {
      const from = item.from?.name || item.from_agent || '';
      return from && !connected.has(from);
    });
  }
  return items.filter((item) => (item.from?.name || item.from_agent || '') === peerName);
}

function connectPeer(name) {
  if (!name) return;
  const next = new Set(currentConfig.connectedPeers || []);
  next.add(name);
  currentConfig.connectedPeers = Array.from(next);
  if (!currentConfig.activePeer) {
    currentConfig.activePeer = name;
    currentConfig.targetAgent = name;
    inboxPeerFilter = name;
  }
  saveConfig().then(async () => {
    await loadPeers();
    renderInboxList();
  });
}

function peerDescription(peer) {
  const parts = [
    peer.type || 'peer',
    peer.role || '',
    peer.status || '',
    peer.transport || '',
  ].filter(Boolean);
  return parts.length ? parts.join(' / ') : 'peer';
}

async function setActivePeer(name) {
  if (!name) return;
  if (!(currentConfig.connectedPeers || []).includes(name)) return;
  currentConfig.activePeer = name;
  currentConfig.targetAgent = name;
  inboxPeerFilter = name;
  await saveConfig();
  await loadPeers();
  renderInboxList();
  showMainView();
}

async function disconnectPeer(name) {
  if (!name) return;
  currentConfig.connectedPeers = (currentConfig.connectedPeers || []).filter((peer) => peer !== name);
  if (currentConfig.activePeer === name) {
    currentConfig.activePeer = currentConfig.connectedPeers[0] || '';
    currentConfig.targetAgent = currentConfig.activePeer;
  }
  if (inboxPeerFilter === name) {
    inboxPeerFilter = currentConfig.activePeer || 'all';
  }
  await saveConfig();
  await loadPeers();
  await loadInbox();
}

function showMainView() {
  $('main-view').classList.add('active');
  $('main-view').classList.remove('hidden');
  $('agents-view').classList.remove('active');
  $('agents-view').classList.add('hidden');
}

async function showAgentsView() {
  $('main-view').classList.remove('active');
  $('main-view').classList.add('hidden');
  $('agents-view').classList.add('active');
  $('agents-view').classList.remove('hidden');
  await loadPeers();
  $('agent-search')?.focus();
}

function openSettings() {
  chrome.tabs.create({ url: chrome.runtime.getURL('popup/settings.html') });
}

function openInbox() {
  chrome.tabs.create({ url: chrome.runtime.getURL('inbox/inbox.html') });
}

function normalizeMessageItem(raw) {
  if (!raw) return raw;
  return {
    id: raw.id,
    chat_id: raw.chat_id,
    from: raw.from || { name: raw.sender_name || raw.sender_id || '未知', type: raw.sender_type || 'agent' },
    content: raw.content || { body: raw.body || '' },
    body: raw.body || raw.content?.body || '',
    subject: raw.subject,
    created_at: raw.created_at,
    delivery: raw.delivery || null,
    recipients: raw.recipients || null,
    attachments: raw.attachments || [],
    _injected: !!raw._injected || !!raw.injected,
  };
}

function escapeHtml(str) {
  return String(str || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function formatTime(iso) {
  if (!iso) return '-';
  const d = new Date(iso);
  return isNaN(d) ? iso : d.toLocaleTimeString('zh-CN', { hour12: false });
}

$('settings-btn').addEventListener('click', openSettings);
$('manage-peers-btn').addEventListener('click', showAgentsView);
$('auto-inject-btn').addEventListener('click', toggleAutoInject);
$('back-main-btn').addEventListener('click', showMainView);
$('refresh-peers-btn').addEventListener('click', loadPeers);
$('agent-search').addEventListener('input', (e) => {
  agentSearchQuery = e.target.value || '';
  renderAgentLists();
});
$('refresh-inbox-btn').addEventListener('click', () => {
  loadInbox();
  refreshStatus();
  loadPeers();
});
$('open-inbox-btn').addEventListener('click', openInbox);
$('reconnect-btn').addEventListener('click', reconnectDaemon);
$('register-btn').addEventListener('click', registerAgent);

document.addEventListener('DOMContentLoaded', load);
