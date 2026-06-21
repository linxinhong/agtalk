// 独立收件箱页面
const $ = (id) => document.getElementById(id);

let currentItems = [];
let currentStatus = 'all';
let selectedId = null;
let autoRefreshTimer = null;

async function load() {
  bindFilters();
  $('refresh-btn').addEventListener('click', refresh);
  await refresh();
  startAutoRefresh();
}

function bindFilters() {
  document.querySelectorAll('.filter').forEach((btn) => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.filter').forEach((b) => b.classList.remove('active'));
      btn.classList.add('active');
      currentStatus = btn.dataset.status;
      renderList();
    });
  });
}

async function refresh() {
  $('refresh-btn').textContent = '刷新中...';
  const status = await sendMessage({ type: 'CHECK_AGTALK_STATUS' });
  const badge = $('connection-status');
  if (status?.connected) {
    badge.className = 'badge online';
    badge.textContent = '在线';
    $('inbox-stats').textContent = `未读 ${status.inboxUnread || 0} / 总计 ${status.inboxTotal || 0} / Peers ${status.peersOnline || 0}`;
  } else {
    badge.className = 'badge offline';
    badge.textContent = '离线';
    $('inbox-stats').textContent = status?.error || '未连接';
  }

  const result = await sendMessage({ type: 'AGTALK_INBOX', status: 'all' });
  if (result?.ok && Array.isArray(result.items)) {
    currentItems = result.items;
  } else {
    // 服务器加载失败时尝试本地缓存
    const local = await sendMessage({ type: 'GET_RECENT_MESSAGES', limit: 200 });
    if (local?.ok && Array.isArray(local.items) && local.items.length > 0) {
      currentItems = local.items.map((msg) => ({
        id: msg.id,
        chat_id: msg.chat_id,
        from: { name: msg.from_name, type: msg.from_type },
        content: { body: msg.body },
        subject: msg.subject,
        created_at: msg.created_at,
        delivery: { status: msg.status, read_at: msg.read_at, done_at: msg.done_at },
        _local: true,
      }));
    } else {
      currentItems = [];
      if (!result?.ok) {
        $('message-list').innerHTML = `<div class="empty">加载失败: ${result?.error || '未知错误'}</div>`;
        $('refresh-btn').textContent = '刷新';
        return;
      }
    }
  }
  renderList();
  $('refresh-btn').textContent = '刷新';
}

function renderList() {
  const list = $('message-list');
  const filtered = filterItems(currentItems, currentStatus);
  if (filtered.length === 0) {
    list.innerHTML = '<div class="empty">无消息</div>';
    return;
  }

  list.innerHTML = filtered.map((item) => {
    const delivery = item.delivery || (item.recipients?.[0] ? { status: item.recipients[0].status, read_at: item.recipients[0].read_at } : {});
    const isUnread = !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
    const cls = ['message-item', isUnread ? 'unread' : '', item.id === selectedId ? 'active' : ''].join(' ');
    const body = item.content?.body || item.body || '';
    const localTag = item._local ? '<span class="local-tag">本地</span> ' : '';
    return `
      <div class="${cls}" data-id="${item.id}">
        <span class="message-sender">${localTag}${item.from?.name || item.from_agent || '未知'}</span>
        <span class="message-preview">${escapeHtml(body).slice(0, 80)}</span>
        <span class="message-time">${formatTime(item.created_at)}</span>
      </div>
    `;
  }).join('');

  list.querySelectorAll('.message-item').forEach((el) => {
    el.addEventListener('click', () => selectMessage(el.dataset.id));
  });
}

function filterItems(items, status) {
  if (status === 'all') return items;
  if (status === 'unread') {
    return items.filter((i) => {
      const delivery = i.delivery || (i.recipients?.[0] ? { status: i.recipients[0].status, read_at: i.recipients[0].read_at } : {});
      return !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
    });
  }
  if (status === 'pending') return items.filter((i) => (i.delivery?.status || i.status) === 'pending');
  if (status === 'action_required') return items.filter((i) => (i.delivery?.status || i.status) === 'action_required');
  return items;
}

function selectMessage(id) {
  selectedId = id;
  renderList();
  const item = currentItems.find((i) => i.id === id);
  if (!item) return;

  const detail = $('message-detail');
  const body = item.content?.body || item.body || '';
  const delivery = item.delivery || (item.recipients?.[0] ? { status: item.recipients[0].status, read_at: item.recipients[0].read_at } : {});
  detail.innerHTML = `
    <div class="detail-meta">
      <div><strong>发件人:</strong> ${item.from?.name || item.from_agent || '未知'}</div>
      <div><strong>时间:</strong> ${formatTime(item.created_at)}</div>
      <div><strong>状态:</strong> ${delivery.status || item.status || 'pending'}</div>
      ${item.subject ? `<div><strong>主题:</strong> ${escapeHtml(item.subject)}</div>` : ''}
    </div>
    <div class="detail-body">${escapeHtml(body)}</div>
    <div class="detail-actions">
      <button class="reply" id="inject-btn">注入对话框</button>
      <button id="done-btn">标记完成</button>
      <button class="reply" id="reply-btn">回复</button>
    </div>
  `;
  $('inject-btn').addEventListener('click', () => injectMessage(item));
  $('done-btn').addEventListener('click', () => markDone(item.id));
  $('reply-btn').addEventListener('click', () => {
    const replyBody = prompt('回复内容:');
    if (replyBody) sendReply(item, replyBody);
  });
}

async function injectMessage(item) {
  const result = await new Promise((resolve) => {
    chrome.runtime.sendMessage({ type: 'DELIVER_TO_ACTIVE_TAB', item }, resolve);
  });
  if (result?.ok) {
    console.log('[Inbox] 已注入消息:', item.id);
  } else {
    alert('注入失败: ' + (result?.error || '未知错误'));
  }
}

async function markDone(msgId) {
  const result = await sendMessage({ type: 'AGTALK_MARK_DONE', msgId });
  if (result?.ok) {
    await refresh();
  } else {
    alert('标记完成失败: ' + (result?.error || '未知错误'));
  }
}

async function sendReply(item, body) {
  const to = item.from?.name || item.from_agent;
  if (!to) return;
  const result = await sendMessage({
    type: 'AGTALK_SEND',
    toAgent: to,
    body,
    replyTo: item.id,
  });
  if (result?.ok) {
    alert('回复成功');
    await refresh();
  } else {
    alert('回复失败: ' + (result?.error || '未知错误'));
  }
}

function startAutoRefresh() {
  if (autoRefreshTimer) clearInterval(autoRefreshTimer);
  autoRefreshTimer = setInterval(refresh, 10000);
}

function sendMessage(message) {
  return new Promise((resolve) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, error: chrome.runtime.lastError.message });
        return;
      }
      resolve(response);
    });
  });
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
  return isNaN(d) ? iso : d.toLocaleString('zh-CN', { hour12: false });
}

document.addEventListener('DOMContentLoaded', load);
