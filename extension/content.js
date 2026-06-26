// 通用 content script：根据当前 URL 匹配平台配置，采集对话并注入 agtalk 消息

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
  activePeer: '',
  connectedPeers: [],
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
let stopButtonPreviouslyPresent = false;
let currentPlatform = null;
let runtimeConfig = { ...DEFAULT_CONFIG };
let peerPickerState = null;
let extensionAlive = true;
let actionButtonUpdateTimer = null;
let captureDebounceTimer = null;
let bootstrapObserver = null;
let navigationHooksInstalled = false;

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

function isContextInvalidatedMessage(message) {
  return typeof message === 'string' && (
    message.includes('Extension context invalidated') ||
    message.includes('Extension context was invalidated')
  );
}

function deactivateExtensionContext(reason) {
  if (!extensionAlive) return;
  extensionAlive = false;
  console.warn('[CS] 扩展上下文已失效，停止 agtalk content script:', reason || '');
  teardownObservers();
  hidePeerPicker();
}

function safeRuntimeSendMessage(message, callback) {
  if (!extensionAlive || typeof chrome === 'undefined' || !chrome.runtime?.id) {
    deactivateExtensionContext('runtime unavailable');
    if (callback) callback({ ok: false, error: 'extension_context_invalidated' });
    return;
  }
  try {
    chrome.runtime.sendMessage(message, (response) => {
      const lastError = chrome.runtime.lastError;
      if (lastError) {
        if (isContextInvalidatedMessage(lastError.message)) {
          deactivateExtensionContext(lastError.message);
        } else {
          console.error('[CS] sendMessage 失败:', lastError.message);
        }
        if (callback) callback({ ok: false, error: lastError.message });
        return;
      }
      if (callback) callback(response);
    });
  } catch (err) {
    if (isContextInvalidatedMessage(err.message)) {
      deactivateExtensionContext(err.message);
    } else {
      console.error('[CS] sendMessage 异常:', err.message);
    }
    if (callback) callback({ ok: false, error: err.message });
  }
}

function loadRuntimeConfig() {
  if (!extensionAlive || typeof chrome === 'undefined' || !chrome.runtime?.id) return;
  try {
    chrome.storage.local.get(['agtalk_config'], (result) => {
      if (result.agtalk_config) {
        runtimeConfig = { ...DEFAULT_CONFIG, ...result.agtalk_config };
      }
    });
  } catch (err) {
    if (isContextInvalidatedMessage(err.message)) {
      deactivateExtensionContext(err.message);
    }
  }
}

try {
  chrome.storage.onChanged.addListener((changes) => {
    if (!extensionAlive) return;
    if (changes.agtalk_config) {
      runtimeConfig = { ...DEFAULT_CONFIG, ...changes.agtalk_config.newValue };
      console.log('[CS] 配置已更新，目标 agent:', runtimeConfig.targetAgent);
    }
  });
} catch (err) {
  if (isContextInvalidatedMessage(err.message)) {
    deactivateExtensionContext(err.message);
  }
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

function escapeHtml(str) {
  return String(str || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function getMessageItems() {
  if (!currentPlatform) return [];
  return Array.from(document.querySelectorAll(currentPlatform.selectors.messageItems));
}

function captureAndSend() {
  setTimeout(() => {
    const items = getMessageItems();
    if (items.length === 0) { resetState(); return; }

    // 锁定最后一轮 assistant 消息，避免抓到历史回答或思考过程
    let aiIndex = items.length - 1;
    while (aiIndex >= 0 && !currentPlatform.isAiMessage(items[aiIndex])) aiIndex--;
    if (aiIndex < 0) { resetState(); return; }
    const aiNode = items[aiIndex];

    const aiText = currentPlatform.extractText(aiNode, false);
    if (!aiText || isAgtalkInjected(aiText)) { resetState(); return; }

    const hash = djb2(aiText);
    if (hash === lastAiHash) { resetState(); return; }
    lastAiHash = hash;

    let userText = '';
    if (aiIndex >= 1) {
      const userNode = items[aiIndex - 1];
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
    safeRuntimeSendMessage({ type: 'CHAT_TURN', payload }, (response) => {
      if (response?.ok === false) return;
      console.log('[CS] 消息已送达 background:', response);
    });

    resetState();
  }, runtimeConfig.captureDelay || 300);
}

function resetState() {
  currentState = STATE.IDLE;
}

function initObserverA() {
  const container = document.querySelector(currentPlatform.selectors.chatContainer) || document.body;
  if (!container) return;
  observerA = new MutationObserver(() => {
    scheduleActionButtonUpdate();
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

function scheduleActionButtonUpdate() {
  if (actionButtonUpdateTimer) return;
  actionButtonUpdateTimer = setTimeout(() => {
    actionButtonUpdateTimer = null;
    if (!extensionAlive || !currentPlatform) return;
    addAgtalkActionButtons();
  }, 150);
}

function teardownObservers() {
  if (observerA) {
    observerA.disconnect();
    observerA = null;
  }
  if (observerB) {
    observerB.disconnect();
    observerB = null;
  }
  if (bootstrapObserver) {
    bootstrapObserver.disconnect();
    bootstrapObserver = null;
  }
  if (actionButtonUpdateTimer) {
    clearTimeout(actionButtonUpdateTimer);
    actionButtonUpdateTimer = null;
  }
  if (captureDebounceTimer) {
    clearTimeout(captureDebounceTimer);
    captureDebounceTimer = null;
  }
  stopButtonPreviouslyPresent = false;
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

    // 找到当前操作栏所属的 assistant 消息容器
    // ChatGPT 操作栏与 assistant 消息是同级兄弟，统一在 .agent-turn 容器内
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

    // 操作栏与 assistant 消息在同一 turn 容器内
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

function sendToAgtalk(text, anchor = null) {
  if (!text) {
    showPeerPickerNotice('没有可发送的内容', '', false, anchor);
    return;
  }
  // 优先使用当前 tab 已关联的 peer 直接发送
  safeRuntimeSendMessage({ type: 'GET_TAB_ASSOCIATION' }, (response) => {
    if (response?.ok && response.peer) {
      safeRuntimeSendMessage({
        type: 'AGTALK_SEND',
        toAgent: response.peer,
        body: text,
      }, (sendResponse) => {
        if (sendResponse?.ok) {
          console.log(`[CS] 已直接发送到关联 peer: target=${sendResponse.to || response.peer}, msg_id=${sendResponse.msg_id || 'undefined'}`);
        } else {
          console.error('[CS] 直接发送失败:', sendResponse?.error);
          if (sendResponse?.error) showPeerPickerNotice('发送失败', sendResponse.error, false, anchor);
        }
      });
      return;
    }
    openPeerPicker(text, anchor);
  });
}

function openPeerPicker(text, anchor = null) {
  safeRuntimeSendMessage({ type: 'GET_CONNECTED_PEERS' }, (response) => {
    if (response?.ok === false) {
      showPeerPickerNotice('无法获取已连接 Peer', response.error || '请重新加载扩展和当前页面', false, anchor);
      return;
    }
    const linkedPeer = response?.recommendedPeer || '';
    const peers = Array.isArray(response?.peers)
      ? response.peers
        .filter((peer) => peer.connected && peer.name && peer.name !== runtimeConfig.agentName)
        .map((peer) => ({ ...peer, linked: peer.name === linkedPeer, recommended: peer.name === linkedPeer }))
      : [];
    if (peers.length === 0) {
      showPeerPickerNotice('没有已连接的 Peer', '请先在收件箱连接一个 Agent', true, anchor);
      return;
    }
    peers.sort((a, b) => Number(!!b.recommended) - Number(!!a.recommended) || a.name.localeCompare(b.name));
    showPeerPicker(text, peers, anchor, linkedPeer);
  });
}

function peerPickerShell(title, bodyHtml, anchor = null) {
  hidePeerPicker();
  const host = document.createElement('div');
  host.id = 'agtalk-peer-picker';
  host.style.cssText = [
    'position:fixed',
    'z-index:2147483647',
    anchor ? 'left:0' : 'right:16px',
    anchor ? 'top:0' : 'bottom:16px',
    'width:320px',
    'max-width:calc(100vw - 32px)',
    'background:#fff',
    'color:#111',
    'border:1px solid rgba(0,0,0,.15)',
    'border-radius:8px',
    'box-shadow:0 10px 24px rgba(0,0,0,.16)',
    'font:12px/1.35 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif',
  ].join(';');
  host.innerHTML = `
    <div style="padding:8px 10px;border-bottom:1px solid rgba(0,0,0,.08);display:flex;justify-content:space-between;gap:6px;align-items:center;">
      <strong style="font-size:12px;">${escapeHtml(title)}</strong>
      <button type="button" data-close style="width:22px;height:22px;border:none;background:transparent;font-size:14px;cursor:pointer;line-height:1;">x</button>
    </div>
    ${bodyHtml}
  `;
  document.body.appendChild(host);
  if (anchor) positionPeerPicker(host, anchor);
  peerPickerState = { host };
  host.querySelector('[data-close]').addEventListener('click', hidePeerPicker);
  document.addEventListener('keydown', onPeerPickerKeyDown, true);
  return host;
}

function positionPeerPicker(host, anchor) {
  const margin = 12;
  const gap = 8;
  const rect = host.getBoundingClientRect();
  const width = rect.width || 320;
  const height = rect.height || 120;
  let left = anchor.x + gap;
  let top = anchor.y + gap;
  if (left + width + margin > window.innerWidth) {
    left = anchor.x - width - gap;
  }
  if (top + height + margin > window.innerHeight) {
    top = anchor.y - height - gap;
  }
  left = Math.max(margin, Math.min(left, window.innerWidth - width - margin));
  top = Math.max(margin, Math.min(top, window.innerHeight - height - margin));
  host.style.left = `${left}px`;
  host.style.top = `${top}px`;
}

function showPeerPickerNotice(title, detail = '', includeOpenInbox = false, anchor = null) {
  const host = peerPickerShell(title, `
    <div style="padding:12px;display:grid;gap:10px;">
      ${detail ? `<div style="color:#555;">${escapeHtml(detail)}</div>` : ''}
      ${includeOpenInbox ? '<button type="button" data-open-inbox style="border:1px solid rgba(0,0,0,.12);background:#0b57d0;color:#fff;border-radius:8px;padding:8px 10px;cursor:pointer;">打开收件箱</button>' : ''}
    </div>
  `, anchor);
  const openBtn = host.querySelector('[data-open-inbox]');
  if (openBtn) {
    openBtn.addEventListener('click', () => {
      safeRuntimeSendMessage({ type: 'OPEN_INBOX' }, () => hidePeerPicker());
    });
  }
}

function showPeerPicker(text, peers, anchor = null, linkedPeer = '') {
  const host = peerPickerShell('发送到 Peer', `
    <div style="padding:6px 10px;border-bottom:1px solid rgba(0,0,0,.08);color:#555;font-size:10px;display:flex;justify-content:space-between;align-items:center;">
      <span>已连接 ${peers.length} 个 Peer</span>
      ${linkedPeer ? `<span data-header-linked style="color:#1a7f37;font-size:9px;">已关联 ${escapeHtml(linkedPeer)}</span>` : ''}
    </div>
    <div style="max-height:220px;overflow:auto;padding:6px;display:grid;gap:4px;">
      ${peers.map((peer) => {
        const isLinked = peer.name === linkedPeer;
        const linkTitle = isLinked ? '已关联当前页面（点击取消）' : '关联当前页面';
        const linkIcon = isLinked ? '🔗' : '🔌';
        const linkColor = isLinked ? '#1a7f37' : '#888';
        return `
        <div data-peer-row="${escapeHtml(peer.name)}" style="display:flex;align-items:stretch;gap:4px;">
          <button type="button" data-peer="${escapeHtml(peer.name)}" style="flex:1;text-align:left;border:1px solid rgba(0,0,0,.12);background:#f8f9fb;border-radius:6px;padding:6px 8px;cursor:pointer;min-height:38px;">
            <div style="display:flex;align-items:center;gap:6px;min-width:0;">
              ${peer.recommended ? '<span style="display:inline-flex;align-items:center;padding:1px 5px;border-radius:999px;background:#e8f0fe;color:#174ea6;font-size:9px;line-height:1;flex:0 0 auto;">推荐</span>' : ''}
              <div style="font-weight:700;font-size:11px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;min-width:0;">${escapeHtml(peer.name)}</div>
            </div>
            <div style="color:#666;font-size:10px;margin-top:1px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">${escapeHtml(formatPeerInfo(peer))}</div>
          </button>
          <button type="button" data-link-peer="${escapeHtml(peer.name)}" title="${linkTitle}" style="flex:0 0 32px;display:flex;align-items:center;justify-content:center;border:1px solid rgba(0,0,0,.12);background:#fff;border-radius:6px;cursor:pointer;font-size:14px;color:${linkColor};">
            ${linkIcon}
          </button>
        </div>
        `;
      }).join('')}
    </div>
  `, anchor);
  peerPickerState = { text, host, linkedPeer };
  host.querySelectorAll('[data-peer]').forEach((btn) => {
    btn.addEventListener('click', () => {
      const toAgent = btn.dataset.peer;
      hidePeerPicker();
      safeRuntimeSendMessage({
        type: 'AGTALK_SEND',
        toAgent,
        body: text,
      }, (response) => {
        if (response?.ok) {
          console.log(`[CS] 已发送到 agtalk: target=${response.to || toAgent}, msg_id=${response.msg_id || 'undefined'}`);
        } else {
          console.error('[CS] 发送到 agtalk 失败:', response?.error);
          if (response?.error) showPeerPickerNotice('发送失败', response.error, false, anchor);
        }
      });
    });
  });
  host.querySelectorAll('[data-link-peer]').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const peer = btn.dataset.linkPeer;
      const isLinked = peer === linkedPeer;
      if (isLinked) {
        // 取消关联：发送空 peer
        safeRuntimeSendMessage({ type: 'ASSOCIATE_TAB_PEER', peer: '' }, (response) => {
          if (response?.ok) {
            refreshPeerPickerLinkState(host, '', peers.map((p) => p.name));
          } else {
            console.error('[CS] 取消关联失败:', response?.error);
          }
        });
      } else {
        safeRuntimeSendMessage({ type: 'ASSOCIATE_TAB_PEER', peer }, (response) => {
          if (response?.ok) {
            refreshPeerPickerLinkState(host, peer, peers.map((p) => p.name));
          } else {
            console.error('[CS] 关联失败:', response?.error);
          }
        });
      }
    });
  });
}

function refreshPeerPickerLinkState(host, linkedPeer, peerNames) {
  if (!host) return;
  peerNames.forEach((name) => {
    const btn = host.querySelector(`[data-link-peer="${CSS.escape(name)}"]`);
    const row = host.querySelector(`[data-peer-row="${CSS.escape(name)}"]`);
    if (!btn) return;
    const isLinked = name === linkedPeer;
    btn.title = isLinked ? '已关联当前页面（点击取消）' : '关联当前页面';
    btn.textContent = isLinked ? '🔗' : '🔌';
    btn.style.color = isLinked ? '#1a7f37' : '#888';
    if (row) row.style.background = isLinked ? '#f6fef9' : '';
  });
  const headerLinked = host.querySelector('[data-header-linked]');
  if (headerLinked) {
    headerLinked.textContent = linkedPeer ? `已关联 ${linkedPeer}` : '';
    headerLinked.style.display = linkedPeer ? 'inline' : 'none';
  }
  if (peerPickerState) peerPickerState.linkedPeer = linkedPeer;
}

function formatPeerInfo(peer) {
  const parts = [
    peer.type || 'peer',
    peer.role || '',
    peer.status || '',
    peer.transport || '',
  ].filter(Boolean);
  return parts.length ? parts.join(' / ') : 'peer';
}

function onPeerPickerKeyDown(e) {
  if (e.key === 'Escape') {
    hidePeerPicker();
  }
}

function hidePeerPicker() {
  if (peerPickerState?.host && peerPickerState.host.parentNode) {
    peerPickerState.host.parentNode.removeChild(peerPickerState.host);
  }
  peerPickerState = null;
  document.removeEventListener('keydown', onPeerPickerKeyDown, true);
}

function initObserverB() {
  observerB = new MutationObserver(() => {
    if (!extensionAlive) return;
    const isPresent = currentPlatform.isStopButtonPresent();
    if (isPresent && !stopButtonPreviouslyPresent && currentState === STATE.IDLE) {
      currentState = STATE.STREAMING;
      console.log('[CS] 进入 STREAMING 状态');
    }
    if (!isPresent && stopButtonPreviouslyPresent && currentState === STATE.STREAMING) {
      console.log('[CS] 停止按钮消失，启动文本稳定检测');
    }
    if (isPresent || stopButtonPreviouslyPresent) {
      if (currentState === STATE.IDLE) {
        currentState = STATE.STREAMING;
      }
      scheduleCaptureCheck();
    }
    stopButtonPreviouslyPresent = isPresent;
  });
  observerB.observe(document.body, { childList: true, subtree: true });
}

function scheduleCaptureCheck() {
  if (captureDebounceTimer) clearTimeout(captureDebounceTimer);
  captureDebounceTimer = setTimeout(() => {
    captureDebounceTimer = null;
    if (!extensionAlive || !currentPlatform || currentState !== STATE.STREAMING) return;
    if (currentPlatform.isStopButtonPresent()) {
      scheduleCaptureCheck();
      return;
    }
    currentState = STATE.COMPLETE;
    captureAndSend();
  }, Math.max(700, runtimeConfig.captureDelay || 300));
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
    safeRuntimeSendMessage({ type: 'AGTALK_MARK_READ', msgId: item.id }, (response) => {
      if (response?.ok === false) return;
      console.log('[CS] 已标记消息已读:', item.id);
    });
  }
}

function setupPlatformObservers() {
  if (!extensionAlive) return false;
  currentPlatform = matchPlatform();
  if (!currentPlatform) {
    console.log('[CS] 当前页面不匹配任何平台');
    return false;
  }
  const container = document.querySelector(currentPlatform.selectors.chatContainer);
  if (!container) {
    return false;
  }
  console.log('[CS] 平台:', currentPlatform.name, '容器已就绪');
  teardownObservers();
  initObserverA();
  initObserverB();
  addAgtalkActionButtons();
  return true;
}

function startBootstrapObserver() {
  if (bootstrapObserver || !extensionAlive) return;
  const root = document.documentElement || document.body;
  if (!root) return;
  bootstrapObserver = new MutationObserver(() => {
    if (!extensionAlive) return;
    if (setupPlatformObservers()) {
      if (bootstrapObserver) {
        bootstrapObserver.disconnect();
        bootstrapObserver = null;
      }
    }
  });
  bootstrapObserver.observe(root, { childList: true, subtree: true });
}

function installNavigationHooks() {
  if (navigationHooksInstalled) return;
  navigationHooksInstalled = true;
  const notify = () => window.dispatchEvent(new Event('agtalk-locationchange'));
  const wrap = (method) => {
    const original = history[method];
    if (typeof original !== 'function' || original.__agtalkWrapped) return;
    const wrapped = function (...args) {
      const result = original.apply(this, args);
      notify();
      return result;
    };
    wrapped.__agtalkWrapped = true;
    history[method] = wrapped;
  };
  wrap('pushState');
  wrap('replaceState');
  window.addEventListener('popstate', notify);
  window.addEventListener('agtalk-locationchange', () => {
    if (!extensionAlive) return;
    teardownObservers();
    resetState();
    if (!setupPlatformObservers()) {
      startBootstrapObserver();
    }
  });
}

function bootstrapContentScript() {
  if (!extensionAlive) return;
  teardownObservers();
  resetState();
  if (!setupPlatformObservers()) {
    startBootstrapObserver();
  }
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!extensionAlive) {
    sendResponse({ ok: false, error: 'extension_context_invalidated' });
    return true;
  }
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

loadRuntimeConfig();
installNavigationHooks();
console.log('[CS] agtalk Web Bridge content script 已加载');
bootstrapContentScript();
safeRuntimeSendMessage({ type: 'REGISTER_AGENT' }, (res) => {
  if (res?.ok === false) return;
  console.log('[CS] 自动注册结果:', res?.ok ? '成功' : (res?.error || '失败'));
});
})();
