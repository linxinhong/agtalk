// 通用 content script：根据当前 URL 匹配平台配置，采集对话并注入 agtalk 消息
let extensionAlive = true;
function isExtensionDead() {
  if (!extensionAlive) return true;
  try {
    if (!chrome || !chrome.runtime || !chrome.runtime.id) { extensionAlive = false; return true; }
    return false;
  } catch (e) { extensionAlive = false; return true; }
}
function safeSendMessage(msg, cb) {
  if (isExtensionDead()) { if (cb) cb({ ok: false, error: 'context_invalidated' }); return; }
  try {
    chrome.runtime.sendMessage(msg, function (resp) {
      if (chrome.runtime.lastError) {
        if ((chrome.runtime.lastError.message || '').includes('context invalidated')) extensionAlive = false;
        if (cb) cb({ ok: false, error: chrome.runtime.lastError.message });
        return;
      }
      if (cb) cb(resp);
    });
  } catch (e) {
    if ((e.message || '').includes('context invalidated')) extensionAlive = false;
    if (cb) cb({ ok: false, error: e.message });
  }
}

(function () {
  if (window.__agtalkBridgeInjected) {
    console.log('[CS] agtalk content script 已存在，跳过重复注入');
    return;
  }
  window.__agtalkBridgeInjected = true;

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
  pollInterval: 5000,
  workspaceRoot: '/virtual/web-bridge',
  workspaceName: 'web-bridge',
  captureDelay: 300,
};

const STATE = { IDLE: 'IDLE', STREAMING: 'STREAMING', COMPLETE: 'COMPLETE' };

let currentState = STATE.IDLE;
let lastAiHash = null;
let observerA = null;
let observerB = null;
let textCheckTimer = null;
let lastAiText = '';
let stableCount = 0;
let stopButtonPreviouslyPresent = false;
let currentPlatform = null;
let runtimeConfig = { ...DEFAULT_CONFIG };

// 所有内置平台（由 manifest 按顺序注入）
const BUILTIN_PLATFORMS = [
  typeof chatgptPlatform !== 'undefined' ? chatgptPlatform : null,
  typeof claudePlatform !== 'undefined' ? claudePlatform : null,
  typeof siderPlatform !== 'undefined' ? siderPlatform : null,
  typeof chatglmPlatform !== 'undefined' ? chatglmPlatform : null,
].filter(Boolean);

function matchPlatform() {
  const href = window.location.href;
  for (const p of BUILTIN_PLATFORMS) {
    if (p.match.some((m) => href.startsWith(m.replace('/*', '')))) return p;
  }
  if (typeof getCustomPlatform === 'function') {
    const custom = getCustomPlatform();
    if (custom && custom.match.some((m) => href.startsWith(m.replace('/*', '')))) return custom;
  }
  return null;
}

function loadRuntimeConfig() {
  if (isExtensionDead()) return;
  chrome.storage.local.get(['agtalk_config'], (result) => {
    if (result.agtalk_config) {
      runtimeConfig = { ...DEFAULT_CONFIG, ...result.agtalk_config };
    }
  });
}

function isVisible(el) {
  if (!el) return false;
  const style = window.getComputedStyle(el);
  return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
}

function djb2(str) {
  let hash = 5381;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) + hash) + str.charCodeAt(i);
  }
  return hash;
}

function parseDirectives(text) {
  const result = { toAgent: null, fromAgent: null, body: text || '' };
  const fmMatch = result.body.match(/^---\s*\n([\s\S]*?)\n---\s*\n([\s\S]*)$/);
  if (fmMatch) {
    const front = fmMatch[1];
    result.body = fmMatch[2].trim();
    const to = front.match(/^to:\s*(.+)$/m);
    const from = front.match(/^(?:from|from_agent):\s*(.+)$/m);
    if (to) result.toAgent = to[1].trim();
    if (from) result.fromAgent = from[1].trim();
    return result;
  }
  const legacy = result.body.match(/\[from:([a-z]+_[a-z]+_[A-Za-z0-9_]+)\]/);
  if (legacy) {
    result.toAgent = legacy[1];
    result.body = result.body.replace(/\[from:[a-z]+_[a-z]+_[A-Za-z0-9_]+\]\s*/, '').trim();
  }
  return result;
}

function extractConversationId() {
  const match = window.location.href.match(/conversation\/([a-zA-Z0-9]+)/);
  return match ? match[1] : String(Date.now());
}

function isAgtalkInjected(text) {
  return typeof text === 'string' && (
    text.includes('from_agent:') ||
    text.includes('msg_id:') ||
    text.includes('channel: agtalk') ||
    text.startsWith('[agtalk 系统已连接]')
  );
}

function getMessageItems() {
  if (!currentPlatform) return [];
  return Array.from(document.querySelectorAll(currentPlatform.selectors.messageItems));
}

function captureAndSend() {
  setTimeout(() => {
    const items = getMessageItems();
    if (items.length === 0) { resetState(); return; }

    const aiNode = items[items.length - 1];
    if (!currentPlatform.isAiMessage(aiNode)) { resetState(); return; }

    const aiText = currentPlatform.extractText(aiNode, false);
    if (!aiText || isAgtalkInjected(aiText)) { resetState(); return; }

    const hash = djb2(aiText);
    if (hash === lastAiHash) { resetState(); return; }
    lastAiHash = hash;

    let userText = '';
    if (items.length >= 2) {
      const userNode = items[items.length - 2];
      if (currentPlatform.isUserMessage(userNode)) {
        userText = currentPlatform.extractText(userNode, true);
      }
    }

    const parsed = parseDirectives(userText);
    const payload = {
      source: runtimeConfig.agentName || `${currentPlatform.id}_web`,
      timestamp: Date.now(),
      conversation_id: extractConversationId(),
      turn: { user: userText, assistant: aiText },
    };
    if (parsed.fromAgent) payload.replyToAgent = parsed.fromAgent;
    if (parsed.toAgent) payload.toAgent = parsed.toAgent;

    console.log('[CS] 准备发送:', payload.turn.user.slice(0, 30), '→', payload.turn.assistant.slice(0, 30));
    safeSendMessage({ type: 'CHAT_TURN', payload }, function (response) {
      console.log('[CS] 消息已送达 background:', response);
    });

    resetState();
  }, runtimeConfig.captureDelay || 300);
}

function resetState() {
  currentState = STATE.IDLE;
}

function initObserverA() {
  const container = document.querySelector(currentPlatform.selectors.chatContainer);
  if (!container) return;
  observerA = new MutationObserver(() => {
    addAgtalkActionButtons();
  });
  observerA.observe(container, { childList: true, subtree: true });
}

function addAgtalkActionButtons() {
  if (!currentPlatform) return;
  if (currentPlatform.id === 'sider') {
    addSiderAgtalkButtons();
  } else if (currentPlatform.id === 'chatglm') {
    addChatglmAgtalkButtons();
  } else if (currentPlatform.id === 'chatgpt') {
    addChatgptAgtalkButtons();
  } else if (currentPlatform.id === 'claude') {
    addClaudeAgtalkButtons();
  }
}

let actionButtonScanTimer = null;
function startActionButtonScan() {
  if (actionButtonScanTimer) return;
  actionButtonScanTimer = setInterval(() => {
    addAgtalkActionButtons();
  }, 2000);
}
function stopActionButtonScan() {
  if (actionButtonScanTimer) {
    clearInterval(actionButtonScanTimer);
    actionButtonScanTimer = null;
  }
}

function addSiderAgtalkButtons() {
  const messages = document.querySelectorAll('.message-inner');
  let added = 0;
  messages.forEach((msgInner) => {
    if (msgInner.dataset.agtalkButtonAdded) return;
    const contentArea = msgInner.querySelector('.content-area');
    if (!contentArea) return; // 只给 AI 消息加

    // 操作栏可能是 msgInner 的兄弟，也可能在父元素的某处
    let actionsRow = msgInner.nextElementSibling?.querySelector('.actions')
      || msgInner.parentElement?.querySelector('.actions')
      || msgInner.closest('.group-hover\\/outer')?.querySelector('.actions')
      || msgInner.parentElement?.parentElement?.querySelector('.actions');

    if (!actionsRow) {
      console.log('[CS] 未找到 actions row，跳过一条消息');
      return;
    }

    const btn = document.createElement('div');
    btn.className = 'action-btn flex-center text-text-primary-3 hover:text-text-primary-2 hover:bg-grey-fill1-hover size-[26px] shrink-0 cursor-pointer rounded-[6px] agtalk-send-btn';
    btn.title = '发送到 agtalk';
    btn.innerHTML = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg" style="display:block;">
      <path d="M1.5 7L12.5 1.5L7 12.5V7H1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" fill="none"/>
    </svg>`;
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const text = extractSiderMessageText(msgInner);
      sendToAgtalk(text, getClickAnchor(e));
    });

    actionsRow.appendChild(btn);
    msgInner.dataset.agtalkButtonAdded = 'true';
    added++;
  });
  if (added > 0) console.log('[CS] Sider 已添加', added, '个 agtalk 按钮');
}

function extractSiderMessageText(msgInner) {
  const answerBox = msgInner.querySelector('.answer-markdown-box');
  if (answerBox) return answerBox.innerText.trim();
  const contentArea = msgInner.querySelector('.content-area');
  return contentArea ? contentArea.innerText.trim() : msgInner.innerText.trim();
}

function addChatglmAgtalkButtons() {
  const answers = document.querySelectorAll('.answer-content');
  let added = 0;
  answers.forEach((answer) => {
    if (answer.dataset.agtalkButtonAdded) return;
    const toolbar = answer.querySelector('.interact-operate');
    if (!toolbar) return;

    const leftPart = toolbar.querySelector('.left-btn-part') || toolbar;
    const btn = document.createElement('div');
    btn.className = 'shim copy canuse el-tooltip__trigger el-tooltip__trigger agtalk-send-btn';
    btn.title = '发送到 agtalk';
    btn.style.cssText = 'display:inline-flex;align-items:center;justify-content:center;width:26px;height:26px;cursor:pointer;margin-left:4px;';
    btn.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 14 14" fill="none" style="display:block;">
      <path d="M1.5 7L12.5 1.5L7 12.5V7H1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" fill="none"/>
    </svg>`;
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const text = currentPlatform.extractText(answer, false);
      sendToAgtalk(text, getClickAnchor(e));
    });

    leftPart.appendChild(btn);
    answer.dataset.agtalkButtonAdded = 'true';
    added++;
  });
  if (added > 0) console.log('[CS] ChatGLM 已添加', added, '个 agtalk 按钮');
}

function addChatgptAgtalkButtons() {
  // ChatGPT 每条 AI 回复下方都有操作栏；中英双语兼容
  const actionBars = document.querySelectorAll('[aria-label="回复操作"], [aria-label="Reply actions"]');
  let added = 0;
  actionBars.forEach((bar) => {
    if (bar.dataset.agtalkButtonAdded) return;

    const turnContainer = bar.closest('.agent-turn');
    const turn = turnContainer
      ? turnContainer.querySelector('[data-message-author-role="assistant"]')
      : bar.parentElement?.previousElementSibling?.matches?.('[data-message-author-role="assistant"]')
        ? bar.parentElement.previousElementSibling
        : null;
    if (!turn) return;

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'text-token-text-secondary hover:bg-token-surface-hover rounded-lg agtalk-send-btn';
    btn.setAttribute('aria-label', '发送到 agtalk');
    btn.title = '发送到 agtalk';
    btn.style.cssText = 'display:inline-flex;align-items:center;justify-content:center;width:32px;height:32px;pointer-events:auto;';
    btn.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 14 14" fill="none" style="display:block;">
      <path d="M1.5 7L12.5 1.5L7 12.5V7H1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" fill="none"/>
    </svg>`;

    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const text = currentPlatform.extractText(turn, false);
      sendToAgtalk(text, getClickAnchor(e));
    });

    bar.appendChild(btn);
    bar.dataset.agtalkButtonAdded = 'true';
    added++;
  });
  if (added > 0) console.log('[CS] ChatGPT 已添加', added, '个 agtalk 按钮');
}

function addClaudeAgtalkButtons() {
  // Claude 操作栏：role="group" aria-label="Message actions"（中英兼容）
  const actionBars = document.querySelectorAll('[role="group"][aria-label="Message actions"], [role="group"][aria-label="消息操作"], [data-testid="message-actions"], [aria-label*="actions"], [aria-label*="操作"]');
  let added = 0;
  actionBars.forEach((bar) => {
    if (bar.dataset.agtalkButtonAdded) return;

    const turn = bar.closest('[data-test-render-count], [data-testid="assistant-message"], [data-is-streaming], article, .group, main');
    if (!turn) return;

    const assistantNode = turn.matches?.('[data-testid="assistant-message"]')
      ? turn
      : turn.querySelector('[data-testid="assistant-message"], [data-is-streaming]');
    const fallbackTextNode = turn.querySelector('.font-claude-message, .font-claude-response, .standard-markdown');

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'agtalk-send-btn';
    btn.setAttribute('aria-label', '发送到 agtalk');
    btn.title = '发送到 agtalk';
    btn.style.cssText = 'display:inline-flex;align-items:center;justify-content:center;width:32px;height:32px;pointer-events:auto;margin-left:4px;border-radius:6px;color:inherit;background:transparent;border:none;cursor:pointer;';
    btn.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 14 14" fill="none" style="display:block;">
      <path d="M1.5 7L12.5 1.5L7 12.5V7H1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" fill="none"/>
    </svg>`;

    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      let text = '';
      if (assistantNode && currentPlatform) {
        text = currentPlatform.extractText(assistantNode, false);
      }
      if (!text && fallbackTextNode) {
        text = fallbackTextNode.innerText.trim();
      }
      sendToAgtalk(text, getClickAnchor(e));
    });

    bar.appendChild(btn);
    bar.dataset.agtalkButtonAdded = 'true';
    added++;
  });
  if (added > 0) console.log('[CS] Claude 已添加', added, '个 agtalk 按钮');
}

function getClickAnchor(event) {
  if (!event) return null;
  return { x: event.clientX, y: event.clientY };
}

function escapeHtml(str) {
  return String(str || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function sendToAgtalk(text, anchor = null) {
  if (!text) {
    showPeerPickerNotice('没有可发送的内容', '', false, anchor);
    return;
  }
  safeSendMessage({ type: 'GET_TAB_ASSOCIATION' }, function (response) {
    if (!response || !response.ok) { openPeerPicker(text, anchor); return; }
    if (response.peer) {
      safeSendMessage({
        type: 'AGTALK_SEND', toAgent: response.peer, body: text,
      }, function (sendResponse) {
        if (sendResponse && sendResponse.ok) {
          console.log('[CS] 已发送到关联 peer:', response.peer);
        } else if (sendResponse && sendResponse.error) {
          showPeerPickerNotice('发送失败', sendResponse.error, false, anchor);
        }
      });
      return;
    }
    openPeerPicker(text, anchor);
  });
}

/* ─── Peer Picker 弹出层 ─── */
var agtalkPeerPickerState = null;

function openPeerPicker(text, anchor) {
  safeSendMessage({ type: 'GET_CONNECTED_PEERS' }, function (response) {
    if (response && response.ok === false) {
      showPeerPickerNotice('无法获取已连接 Peer', response.error || '请重新加载', false, anchor);
      return;
    }
    var linkedPeer = (response && response.recommendedPeer) ? response.recommendedPeer : '';
    var all = Array.isArray(response && response.peers) ? response.peers : [];
    var peers = all.filter(function (p) {
      return p.connected && p.name && p.name !== runtimeConfig.agentName;
    }).map(function (p) {
      p.linked = p.name === linkedPeer;
      return p;
    });
    if (peers.length === 0) {
      showPeerPickerNotice('没有已连接的 Peer', '请先在扩展中连接 Agent', true, anchor);
      return;
    }
    peers.sort(function (a, b) { return (b.linked ? 1 : 0) - (a.linked ? 1 : 0) || a.name.localeCompare(b.name); });
    showPeerPicker(text, peers, anchor, linkedPeer);
  });
}

function peerPickerShell(title, bodyHtml, anchor) {
  hidePeerPicker();
  var host = document.createElement('div');
  host.id = 'agtalk-peer-picker';
  host.style.cssText = 'position:fixed;z-index:2147483647;' +
    (anchor ? 'left:0;top:0;' : 'right:16px;bottom:16px;') +
    'width:320px;max-width:calc(100vw - 32px);' +
    'background:#fff;color:#111;border:1px solid rgba(0,0,0,.15);border-radius:8px;' +
    'box-shadow:0 10px 24px rgba(0,0,0,.16);' +
    'font:12px/1.35 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;';
  host.innerHTML = '<div style="padding:8px 10px;border-bottom:1px solid rgba(0,0,0,.08);display:flex;justify-content:space-between;align-items:center;">' +
    '<strong style="font-size:12px;">' + escapeHtml(title) + '</strong>' +
    '<button type="button" data-close style="width:22px;height:22px;border:none;background:transparent;font-size:14px;cursor:pointer;">x</button>' +
    '</div>' + bodyHtml;
  document.body.appendChild(host);
  if (anchor) positionPeerPicker(host, anchor);
  agtalkPeerPickerState = { host: host };
  host.querySelector('[data-close]').addEventListener('click', hidePeerPicker);
  document.addEventListener('keydown', onPeerPickerKeyDown, true);
  document.addEventListener('click', onPeerPickerOutsideClick, true);
  return host;
}

function positionPeerPicker(host, anchor) {
  var margin = 12, gap = 8;
  var rect = host.getBoundingClientRect();
  var w = rect.width || 320, h = rect.height || 120;
  var left = anchor.x + gap, top = anchor.y + gap;
  if (left + w + margin > window.innerWidth) left = anchor.x - w - gap;
  if (top + h + margin > window.innerHeight) top = anchor.y - h - gap;
  left = Math.max(margin, Math.min(left, window.innerWidth - w - margin));
  top = Math.max(margin, Math.min(top, window.innerHeight - h - margin));
  host.style.left = left + 'px';
  host.style.top = top + 'px';
}

function showPeerPickerNotice(title, detail, includeOpenInbox, anchor) {
  var body = '<div style="padding:12px;display:grid;gap:10px;">' +
    (detail ? '<div style="color:#555;">' + escapeHtml(detail) + '</div>' : '') +
    (includeOpenInbox ? '<button type="button" data-open-inbox style="border:1px solid rgba(0,0,0,.12);background:#0b57d0;color:#fff;border-radius:8px;padding:8px 10px;cursor:pointer;">打开收件箱</button>' : '') +
    '</div>';
  var host = peerPickerShell(title, body, anchor);
  var btn = host.querySelector('[data-open-inbox]');
  if (btn) btn.addEventListener('click', function () {
    safeSendMessage({ type: 'OPEN_INBOX' }, function () { hidePeerPicker(); });
  });
}

function showPeerPicker(text, peers, anchor, linkedPeer) {
  var html = '<div style="padding:6px 10px;border-bottom:1px solid rgba(0,0,0,.08);color:#555;font-size:10px;display:flex;justify-content:space-between;">' +
    '<span>已连接 ' + peers.length + ' 个 Agent</span>' +
    (linkedPeer ? '<span data-header-linked style="color:#1a7f37;font-size:9px;">已关联 ' + escapeHtml(linkedPeer) + '</span>' : '') +
    '</div>' +
    '<div style="max-height:220px;overflow:auto;padding:6px;display:grid;gap:4px;">';

  peers.forEach(function (peer) {
    var isLinked = peer.name === linkedPeer;
    var linkIcon = isLinked ? '🔗' : '🔌';
    var linkColor = isLinked ? '#1a7f37' : '#888';
    var linkTitle = isLinked ? '已关联当前页面（点击取消）' : '关联当前页面';
    html += '<div data-peer-row="' + escapeHtml(peer.name) + '" style="display:flex;align-items:stretch;gap:4px;">' +
      '<button type="button" data-peer="' + escapeHtml(peer.name) + '" style="flex:1;text-align:left;border:1px solid rgba(0,0,0,.12);background:#f8f9fb;border-radius:6px;padding:6px 8px;cursor:pointer;">' +
      '<div style="display:flex;align-items:center;gap:6px;">' +
      (peer.active ? '<span style="padding:1px 5px;border-radius:999px;background:#e8f0fe;color:#174ea6;font-size:9px;">当前</span>' : '') +
      '<div style="font-weight:700;font-size:11px;">' + escapeHtml(peer.name) + '</div>' +
      '</div><div style="color:#666;font-size:10px;margin-top:1px;">' + escapeHtml(formatPeerInfo(peer)) + '</div>' +
      '</button>' +
      '<button type="button" data-link-peer="' + escapeHtml(peer.name) + '" title="' + linkTitle + '" style="flex:0 0 32px;display:flex;align-items:center;justify-content:center;border:1px solid rgba(0,0,0,.12);background:#fff;border-radius:6px;cursor:pointer;font-size:14px;color:' + linkColor + ';">' + linkIcon + '</button>' +
      '</div>';
  });
  html += '</div>';

  var host = peerPickerShell('发送到 Agent', html, anchor);
  agtalkPeerPickerState = { text: text, host: host, linkedPeer: linkedPeer };

  host.querySelectorAll('[data-peer]').forEach(function (btn) {
    btn.addEventListener('click', function () {
      var toAgent = btn.dataset.peer;
      hidePeerPicker();
      safeSendMessage({
        type: 'AGTALK_SEND', toAgent: toAgent, body: text,
      }, function (resp) {
        if (resp && resp.ok) console.log('[CS] 已发送到 agtalk:', toAgent);
        else if (resp && resp.error) showPeerPickerNotice('发送失败', resp.error, false, anchor);
      });
    });
  });

  host.querySelectorAll('[data-link-peer]').forEach(function (btn) {
    btn.addEventListener('click', function (e) {
      e.stopPropagation();
      var peer = btn.dataset.linkPeer;
      var newPeer = (peer === linkedPeer) ? '' : peer;
      safeSendMessage({ type: 'ASSOCIATE_TAB_PEER', peer: newPeer }, function (resp) {
        if (resp && resp.ok) {
          refreshPeerPickerLinkState(host, newPeer, peers.map(function (p) { return p.name; }));
        }
      });
    });
  });
}

function refreshPeerPickerLinkState(host, linkedPeer, peerNames) {
  if (!host) return;
  peerNames.forEach(function (name) {
    var btn = host.querySelector('[data-link-peer="' + CSS.escape(name) + '"]');
    if (!btn) return;
    var isLinked = name === linkedPeer;
    btn.title = isLinked ? '已关联当前页面（点击取消）' : '关联当前页面';
    btn.textContent = isLinked ? '🔗' : '🔌';
    btn.style.color = isLinked ? '#1a7f37' : '#888';
    var row = host.querySelector('[data-peer-row="' + CSS.escape(name) + '"]');
    if (row) row.style.background = isLinked ? '#f6fef9' : '';
  });
  var headerLinked = host.querySelector('[data-header-linked]');
  if (headerLinked) {
    headerLinked.textContent = linkedPeer ? '已关联 ' + linkedPeer : '';
    headerLinked.style.display = linkedPeer ? 'inline' : 'none';
  }
  if (agtalkPeerPickerState) agtalkPeerPickerState.linkedPeer = linkedPeer;
}

function formatPeerInfo(peer) {
  var parts = [peer.type, peer.role, peer.status, peer.transport].filter(Boolean);
  return parts.length ? parts.join(' / ') : 'peer';
}

function hidePeerPicker() {
  if (agtalkPeerPickerState && agtalkPeerPickerState.host && agtalkPeerPickerState.host.parentNode) {
    agtalkPeerPickerState.host.parentNode.removeChild(agtalkPeerPickerState.host);
  }
  agtalkPeerPickerState = null;
  document.removeEventListener('keydown', onPeerPickerKeyDown, true);
  document.removeEventListener('click', onPeerPickerOutsideClick, true);
}

function onPeerPickerKeyDown(e) {
  if (e.key === 'Escape') hidePeerPicker();
}

function onPeerPickerOutsideClick(e) {
  if (!agtalkPeerPickerState || !agtalkPeerPickerState.host) return;
  if (!agtalkPeerPickerState.host.contains(e.target)) hidePeerPicker();
}

function initObserverB() {
  observerB = new MutationObserver(() => {
    const isPresent = currentPlatform.isStopButtonPresent();
    if (isPresent && !stopButtonPreviouslyPresent && currentState === STATE.IDLE) {
      currentState = STATE.STREAMING;
      console.log('[CS] 进入 STREAMING 状态');
      startTextCheck();
    }
    if (!isPresent && stopButtonPreviouslyPresent && currentState === STATE.STREAMING) {
      console.log('[CS] 停止按钮消失，启动文本稳定检测');
      startTextCheck();
    }
    stopButtonPreviouslyPresent = isPresent;
  });
  observerB.observe(document.body, { childList: true, subtree: true });
}

function startTextCheck() {
  if (textCheckTimer) clearInterval(textCheckTimer);
  lastAiText = '';
  stableCount = 0;

  textCheckTimer = setInterval(() => {
    if (currentState !== STATE.STREAMING) {
      clearInterval(textCheckTimer);
      textCheckTimer = null;
      return;
    }
    const items = getMessageItems();
    if (items.length === 0) return;
    const latest = items[items.length - 1];
    if (!currentPlatform.isAiMessage(latest)) return;
    const currentText = currentPlatform.extractText(latest, false);
    if (currentText === lastAiText && currentText.length > 0) {
      stableCount++;
      if (stableCount >= 5) {
        clearInterval(textCheckTimer);
        textCheckTimer = null;
        currentState = STATE.COMPLETE;
        captureAndSend();
      }
    } else {
      lastAiText = currentText;
      stableCount = 0;
    }
  }, 800);
}

async function handleAgtalkIncoming(item) {
  if (!item || !item.content) return;
  const body = item.content.body || item.body || '';
  const from = item.from?.name || item.from_agent || '';
  const shortId = (item.id || '').slice(0, 8);
  const text = '---\nmsg_id: ' + shortId +
    '\nfrom_agent: ' + from +
    '\nsubject: ' + (item.subject || '') +
    '\nmsg_type: ' + (item.kind || 'text') +
    '\ncreated_at: ' + (item.created_at || '') +
    '\nreply_to_msg_id: ' + (item.reply_to_id || 'null') +
    '\n---\n' + body;

  const result = await currentPlatform.injectText(text);
  console.log('[CS] agtalk 消息注入结果:', result);

  // 注入成功后标记已读（后台 autoInject 已尝试标记，这里是二次确认）
  if (result?.success || result?.ok) {
    safeSendMessage({ type: 'AGTALK_MARK_READ', msgId: item.id }, function () {
      console.log('[CS] 已标记消息已读:', item.id);
    });
  }
}

function waitForContainer() {
  let attempts = 0;
  const timer = setInterval(() => {
    attempts++;
    currentPlatform = matchPlatform();
    if (!currentPlatform) {
      clearInterval(timer);
      console.log('[CS] 当前页面不匹配任何平台');
      return;
    }
    const container = document.querySelector(currentPlatform.selectors.chatContainer);
    if (container) {
      clearInterval(timer);
      console.log('[CS] 平台:', currentPlatform.name, '容器已就绪');
      initObserverA();
      initObserverB();
      startActionButtonScan();
      watchUrlChange();
      return;
    }
    if (attempts >= 60) {
      clearInterval(timer);
      console.error('[CS] 容器加载超时');
    }
  }, 500);
}

function watchUrlChange() {
  let lastUrl = window.location.href;
  const timer = setInterval(() => {
    if (window.location.href !== lastUrl) {
      lastUrl = window.location.href;
      if (observerA) observerA.disconnect();
      if (observerB) observerB.disconnect();
      stopActionButtonScan();
      if (textCheckTimer) { clearInterval(textCheckTimer); textCheckTimer = null; }
      resetState();
      stopButtonPreviouslyPresent = false;
      lastAiText = '';
      stableCount = 0;
      waitForContainer();
    }
  }, 500);
  window._urlWatchInterval = timer;
}

try {
  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === 'PING') {
    sendResponse({ pong: true, platform: currentPlatform?.id || null });
    return true;
  }
  if (message.type === 'SIMULATE_SEND') {
    if (!currentPlatform) {
      sendResponse({ success: false, error: '当前页面不匹配任何平台' });
      return true;
    }
    currentPlatform.injectText(message.text).then((result) => sendResponse(result));
    return true;
  }
  if (message.type === 'AGTALK_INCOMING') {
    if (!currentPlatform) {
      sendResponse({ ok: false, error: '当前页面不匹配任何平台' });
      return true;
    }
    handleAgtalkIncoming(message.item).then(() => sendResponse({ ok: true }));
    return true;
  }
  });
} catch (e) { extensionAlive = false; }

loadRuntimeConfig();
console.log('[CS] agtalk Web Bridge content script 已加载');
waitForContainer();
safeSendMessage({ type: 'REGISTER_AGENT' }, function (res) {
  console.log('[CS] 自动注册结果:', res?.ok ? '成功' : (res?.error || '失败'));
});
})();
