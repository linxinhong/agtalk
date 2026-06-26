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

async function load() {
  await ensureConfig();
  renderAutoInjectButton();
  await refreshStatus();
  await loadLocalInbox(); // 先显示本地缓存，实现秒开
  await loadInbox();      // 再从服务器刷新
  await loadPeers();
  if (currentConfig.enabled && currentConfig.agentName) {
    await registerAgent();
  }
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
    _local: true,
  }));
  renderInboxList('<li class="empty">本地缓存</li>');
}

async function ensureConfig() {
  const result = await chrome.storage.local.get(['agtalk_config']);
  if (result.agtalk_config) {
    currentConfig = normalizeConfig({ ...DEFAULT_CONFIG, ...result.agtalk_config });
  } else {
    currentConfig = { ...DEFAULT_CONFIG };
    await saveConfig();
  }
}

async function saveConfig() {
  await chrome.storage.local.set({ agtalk_config: currentConfig });
  return new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'SAVE_CONFIG', config: currentConfig }, resolve);
  });
}

async function registerAgent() {
  if (!currentConfig.agentName) return;
  return new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'REGISTER_AGENT' }, resolve);
  });
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

function renderInboxList(emptyHint) {
  const list = $('inbox-list');
  if (inboxItems.length === 0) {
    list.innerHTML = emptyHint || '<li class="empty">暂无消息</li>';
    return;
  }

  list.innerHTML = inboxItems.map((item) => {
    const delivery = item.delivery || (item.recipients?.[0] ? { status: item.recipients[0].status, read_at: item.recipients[0].read_at } : {});
    const isUnread = !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
    const body = item.content?.body || item.body || '';
    const shortBody = escapeHtml(body).slice(0, 80);
    const localTag = item._local ? '<span class="local-tag">本地</span> ' : '';
    return `
      <li class="inbox-item ${isUnread ? 'unread' : ''}" data-id="${item.id}">
        <div class="inbox-meta">
          <span class="from">${localTag}${item.from?.name || '未知'}</span>
          <span class="time">${formatTime(item.created_at)}</span>
        </div>
        <div class="preview">${shortBody}</div>
        <div class="detail-body">${escapeHtml(body)}</div>
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
}

async function injectMessage(msgId) {
  const item = inboxItems.find((i) => i.id === msgId);
  if (!item) return;
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'DELIVER_TO_ACTIVE_TAB', item }, resolve);
  });
  if (result?.ok) {
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
  const select = $('target-peer-select');
  const input = $('target-agent');
  if (!result?.ok || !Array.isArray(result.peers)) {
    peers = [];
    return;
  }
  // 同步 connectedPeers 列表
  const connected = new Set(currentConfig.connectedPeers || []);
  result.peers.forEach(function (p) {
    if (p.name && p.connected && p.name !== currentConfig.agentName) connected.add(p.name);
  });
  currentConfig.connectedPeers = Array.from(connected);
  if (!currentConfig.activePeer && currentConfig.connectedPeers.length > 0) {
    currentConfig.activePeer = currentConfig.connectedPeers[0];
    currentConfig.targetAgent = currentConfig.activePeer;
  }
  await saveConfig();
  peers = result.peers.filter((p) => p.name !== currentConfig.agentName);

  const currentValue = select.value || currentConfig.targetAgent || '';
  select.innerHTML = '<option value="">手动输入 / 选择</option>';
  peers.forEach((p) => {
    const option = document.createElement('option');
    option.value = p.name;
    option.textContent = `${p.type === 'web' ? '[web] ' : ''}${p.name} (${p.role || 'peer'})`;
    select.appendChild(option);
  });

  if (currentValue && peers.some((p) => p.name === currentValue)) {
    select.value = currentValue;
  } else if (currentConfig.targetAgent) {
    input.value = currentConfig.targetAgent;
  }
  renderPeerSummary();
}

function onPeerSelectChange() {
  const select = $('target-peer-select');
  const input = $('target-agent');
  if (select.value) {
    input.value = select.value;
    currentConfig.targetAgent = select.value;
    if (!currentConfig.connectedPeers.includes(select.value)) {
      currentConfig.connectedPeers.push(select.value);
    }
    currentConfig.activePeer = select.value;
    saveConfig();
    renderPeerSummary();
  }
}

function onTargetInputChange() {
  currentConfig.targetAgent = $('target-agent').value.trim();
  saveConfig();
}

function renderPeerSummary() {
  const manageBtn = $('manage-peers-btn');
  if (manageBtn) {
    const count = (currentConfig.connectedPeers || []).length;
    const active = currentConfig.activePeer || '无';
    manageBtn.title = count > 0 ? ('Agent 管理: ' + active + ' · 已连接 ' + count) : 'Agent 管理';
  }
}

let allPeersCache = [];

function showMainView() {
  $('main-view').classList.add('active');
  $('main-view').classList.remove('hidden');
  $('agents-view').classList.remove('active');
  $('agents-view').classList.add('hidden');
}

function showAgentsView() {
  $('main-view').classList.remove('active');
  $('main-view').classList.add('hidden');
  $('agents-view').classList.add('active');
  $('agents-view').classList.remove('hidden');
  loadAgentLists();
}

async function loadAgentLists() {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'GET_CONNECTED_PEERS' }, resolve);
  });
  if (!result?.ok || !Array.isArray(result.peers)) return;
  allPeersCache = result.peers.filter(function (p) {
    return p.name && p.name !== currentConfig.agentName;
  });
  const connectedSet = new Set(currentConfig.connectedPeers || []);
  const connected = allPeersCache.filter(function (p) { return connectedSet.has(p.name); });
  const available = allPeersCache.filter(function (p) { return !connectedSet.has(p.name); });
  renderConnectedPeers(connected);
  renderAvailablePeers(available);
  const count = currentConfig.connectedPeers ? currentConfig.connectedPeers.length : 0;
  $('agents-active-label').textContent = count > 0
    ? ('当前: ' + (currentConfig.activePeer || '无') + ' · 已连接 ' + count)
    : '无 active peer';
}

function renderConnectedPeers(connected) {
  const container = $('connected-peer-list');
  if (!connected || connected.length === 0) {
    container.innerHTML = '<div class="empty-inline">尚未连接 Agent</div>';
    return;
  }
  container.innerHTML = connected.map(function (p) {
    var active = p.name === currentConfig.activePeer;
    return '<div class="peer-row' + (active ? ' active' : '') + '" data-name="' + escapeHtml(p.name) + '">' +
      '<div class="peer-main">' +
      '<span class="peer-name">' + escapeHtml(p.name) + '</span>' +
      '<span class="peer-role">' + escapeHtml(peerDescription(p)) + '</span>' +
      '</div>' +
      '<div class="peer-actions">' +
      '<button class="peer-btn set-active" data-name="' + escapeHtml(p.name) + '">' + (active ? '当前' : '设为当前') + '</button>' +
      '<button class="peer-btn disconnect" data-name="' + escapeHtml(p.name) + '">移除</button>' +
      '</div></div>';
  }).join('');
  container.querySelectorAll('.set-active').forEach(function (btn) {
    btn.addEventListener('click', function () { setActivePeer(btn.dataset.name); });
  });
  container.querySelectorAll('.disconnect').forEach(function (btn) {
    btn.addEventListener('click', function () { disconnectPeer(btn.dataset.name); });
  });
}

function renderAvailablePeers(available) {
  const container = $('available-peer-list');
  if (!available || available.length === 0) {
    container.innerHTML = '<div class="empty-inline">没有可连接的 Agent</div>';
    return;
  }
  container.innerHTML = available.map(function (p) {
    return '<div class="peer-row" data-name="' + escapeHtml(p.name) + '">' +
      '<div class="peer-main">' +
      '<span class="peer-name">' + escapeHtml(p.name) + '</span>' +
      '<span class="peer-role">' + escapeHtml(peerDescription(p)) + '</span>' +
      '</div>' +
      '<div class="peer-actions">' +
      '<button class="peer-btn connect-peer" data-name="' + escapeHtml(p.name) + '">连接</button>' +
      '</div></div>';
  }).join('');
  container.querySelectorAll('.connect-peer').forEach(function (btn) {
    btn.addEventListener('click', function () { connectPeer(btn.dataset.name); });
  });
}

function connectPeer(name) {
  if (!name) return;
  if (!currentConfig.connectedPeers.includes(name)) {
    currentConfig.connectedPeers.push(name);
  }
  if (!currentConfig.activePeer) {
    currentConfig.activePeer = name;
    currentConfig.targetAgent = name;
  }
  saveConfig().then(function () {
    loadAgentLists();
    renderPeerSummary();
    loadPeers();
  });
}

async function setActivePeer(name) {
  if (!name || !currentConfig.connectedPeers.includes(name)) return;
  currentConfig.activePeer = name;
  currentConfig.targetAgent = name;
  await saveConfig();
  loadAgentLists();
  renderPeerSummary();
  loadPeers();
  showMainView();
}

async function disconnectPeer(name) {
  if (!name) return;
  currentConfig.connectedPeers = currentConfig.connectedPeers.filter(function (p) { return p !== name; });
  if (currentConfig.activePeer === name) {
    currentConfig.activePeer = currentConfig.connectedPeers[0] || '';
    currentConfig.targetAgent = currentConfig.activePeer;
  }
  await saveConfig();
  loadAgentLists();
  renderPeerSummary();
  loadPeers();
}

function peerDescription(peer) {
  var parts = [peer.type, peer.role, peer.status, peer.transport].filter(Boolean);
  return parts.length ? parts.join(' / ') : 'peer';
}

function openSettings() {
  chrome.tabs.create({ url: chrome.runtime.getURL('popup/settings.html') });
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
$('auto-inject-btn').addEventListener('click', toggleAutoInject);
$('manage-peers-btn').addEventListener('click', showAgentsView);
$('refresh-inbox-btn').addEventListener('click', () => {
  loadInbox();
  refreshStatus();
});
$('target-peer-select').addEventListener('change', onPeerSelectChange);
$('target-agent').addEventListener('change', onTargetInputChange);
$('reconnect-btn').addEventListener('click', reconnectDaemon);
$('back-main-btn').addEventListener('click', showMainView);
$('refresh-peers-btn').addEventListener('click', loadAgentLists);

document.addEventListener('DOMContentLoaded', load);
function normalizeConfig(cfg) {
  const merged = { ...DEFAULT_CONFIG, ...(cfg || {}) };
  const connectedPeers = Array.isArray(merged.connectedPeers)
    ? merged.connectedPeers.map(function (p) { return String(p).trim(); }).filter(Boolean)
    : [];
  if (merged.targetAgent && connectedPeers.indexOf(merged.targetAgent) < 0) {
    connectedPeers.unshift(merged.targetAgent);
  }
  merged.connectedPeers = Array.from(new Set(connectedPeers));
  if (merged.activePeer && merged.connectedPeers.indexOf(merged.activePeer) < 0) {
    merged.activePeer = '';
  }
  if (!merged.activePeer && merged.connectedPeers.length > 0) {
    merged.activePeer = merged.connectedPeers[0];
    merged.targetAgent = merged.activePeer;
  }
  return merged;
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
