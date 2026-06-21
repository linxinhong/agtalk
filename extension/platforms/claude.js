// Claude.ai 平台适配
const claudePlatform = {
  id: 'claude',
  name: 'Claude',
  match: ['https://claude.ai/*'],

  selectors: {
    chatContainer: '.flex-1.flex.flex-col.relative main',
    messageItems: '[data-testid="user-message"], [data-testid="assistant-message"]',
    userMessageAttr: 'user-message',
    aiMessageAttr: 'assistant-message',
    messageText: '.font-claude-message, .whitespace-pre-wrap',
    inputBox: '[contenteditable="true"][data-testid="chat-input"], [contenteditable="true"][data-placeholder*="message"]',
    sendButton: '[data-testid="send-button"], button[class*="send"]:not([disabled])',
    stopButton: '[data-testid="stop-button"]',
  },

  isUserMessage(node) {
    return node.getAttribute('data-testid') === 'user-message';
  },

  isAiMessage(node) {
    return node.getAttribute('data-testid') === 'assistant-message';
  },

  extractText(node, isUser) {
    const area = node.querySelector(this.selectors.messageText);
    return area ? area.innerText.trim() : node.innerText.trim();
  },

  injectText(text) {
    const input = document.querySelector(this.selectors.inputBox);
    if (!input) return { success: false, error: '未找到输入框' };

    input.focus();
    input.click();

    // Claude 使用 ProseMirror，优先用 execCommand 插入文本
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(input);
    range.deleteContents();
    selection.removeAllRanges();
    selection.addRange(range);
    document.execCommand('insertText', false, text);

    // 移除 placeholder 相关 class/attribute
    const p = input.querySelector('p.is-empty, p.is-editor-empty');
    if (p) {
      p.classList.remove('is-empty', 'is-editor-empty');
      p.removeAttribute('data-placeholder');
    }

    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    input.dispatchEvent(new Event('keyup', { bubbles: true }));

    return new Promise((resolve) => {
      setTimeout(() => {
        const sendBtn = document.querySelector(this.selectors.sendButton);
        if (sendBtn && !sendBtn.disabled && sendBtn.offsetParent !== null) {
          sendBtn.click();
          resolve({ success: true, method: 'click' });
        } else {
          input.dispatchEvent(new KeyboardEvent('keydown', {
            key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
            bubbles: true, cancelable: true, shiftKey: false,
          }));
          input.dispatchEvent(new KeyboardEvent('keyup', {
            key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
            bubbles: true, cancelable: true, shiftKey: false,
          }));
          resolve({ success: true, method: 'enter' });
        }
      }, 300);
    });
  },

  isStopButtonPresent() {
    const btn = document.querySelector(this.selectors.stopButton);
    return !!(btn && btn.offsetParent !== null);
  },
};

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { claudePlatform };
}
