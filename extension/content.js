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
    chrome.runtime.sendMessage({ type: 'CHAT_TURN', payload }, (response) => {
      if (chrome.runtime.lastError) {
        if (!chrome.runtime.lastError.message.includes('context invalidated')) {
          console.error('[CS] sendMessage 失败:', chrome.runtime.lastError.message);
        }
        return;
      }
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
      sendToAgtalk(text);
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
      sendToAgtalk(text);
    });

    leftPart.appendChild(btn);
    answer.dataset.agtalkButtonAdded = 'true';
    added++;
  });
  if (added > 0) console.log('[CS] ChatGLM 已添加', added, '个 agtalk 按钮');
}

function sendToAgtalk(text) {
  if (!text) {
    alert('没有可发送的内容');
    return;
  }
  if (!runtimeConfig.targetAgent) {
    alert('未设置目标 agtalk Agent，请在扩展 popup 设置中选择目标 Peer');
    return;
  }
  chrome.runtime.sendMessage({
    type: 'AGTALK_SEND',
    toAgent: runtimeConfig.targetAgent,
    body: text,
  }, (response) => {
    if (chrome.runtime.lastError) {
      console.error('[CS] 发送到 agtalk 失败:', chrome.runtime.lastError.message);
      return;
    }
    if (response?.ok) {
      console.log('[CS] 已发送到 agtalk:', response.msg_id);
    } else {
      console.error('[CS] 发送到 agtalk 失败:', response?.error);
    }
  });
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
    chrome.runtime.sendMessage({ type: 'AGTALK_MARK_READ', msgId: item.id }, () => {
      if (chrome.runtime.lastError) return;
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

loadRuntimeConfig();
console.log('[CS] agtalk Web Bridge content script 已加载');
waitForContainer();
chrome.runtime.sendMessage({ type: 'REGISTER_AGENT' }, (res) => {
  if (chrome.runtime.lastError) return;
  console.log('[CS] 自动注册结果:', res?.ok ? '成功' : (res?.error || '失败'));
});
})();
