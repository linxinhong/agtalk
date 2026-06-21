// ChatGPT (chatgpt.com) 平台适配
const chatgptPlatform = {
  id: 'chatgpt',
  name: 'ChatGPT',
  match: ['https://chatgpt.com/*'],

  selectors: {
    chatContainer: 'main [role="presentation"] > div > div > div',
    messageItems: '[data-testid^="conversation-turn-"]',
    userMessageAttr: 'user',
    aiMessageAttr: 'assistant',
    messageText: '.markdown, [data-message-content]',
    inputBox: '#prompt-textarea',
    sendButton: '[data-testid="send-button"]',
    stopButton: '[data-testid="stop-button"]',
  },

  isUserMessage(node) {
    return !!node.querySelector('[data-message-author-role="user"]');
  },

  isAiMessage(node) {
    return !!node.querySelector('[data-message-author-role="assistant"]');
  },

  extractText(node, isUser) {
    const area = node.querySelector(this.selectors.messageText);
    if (area) return area.innerText.trim();
    const fallback = node.querySelector('[data-message-author-role] > div');
    return fallback ? fallback.innerText.trim() : node.innerText.trim();
  },

  injectText(text) {
    const input = document.querySelector(this.selectors.inputBox);
    if (!input) return { success: false, error: '未找到输入框' };

    input.focus();
    input.click();

    // ChatGPT 使用 ProseMirror contenteditable，尝试多种注入方式
    const editable = input.isContentEditable ? input : input.querySelector('[contenteditable="true"]');
    if (editable) {
      // 方式1：execCommand 插入文本（最兼容 ProseMirror）
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(editable);
      range.deleteContents();
      selection.removeAllRanges();
      selection.addRange(range);
      document.execCommand('insertText', false, text);

      // 清除 placeholder 标记
      const p = editable.querySelector('p[data-empty-paragraph]');
      if (p) {
        p.removeAttribute('data-empty-paragraph');
        p.classList.remove('placeholder');
      }
    } else {
      // 方式2：兜底，直接操作内部 paragraph
      const p = input.querySelector('p');
      if (p) {
        p.innerHTML = '';
        p.appendChild(document.createTextNode(text));
        p.removeAttribute('data-empty-paragraph');
        p.classList.remove('placeholder');
      } else {
        // 方式3：旧版 textarea 兜底
        const descriptor = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
        if (descriptor && descriptor.set) {
          descriptor.set.call(input, text);
        } else {
          input.value = text;
        }
      }
    }

    // 触发事件让 React/ProseMirror 识别变化
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
            bubbles: true, cancelable: true,
          }));
          input.dispatchEvent(new KeyboardEvent('keyup', {
            key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
            bubbles: true, cancelable: true,
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
  module.exports = { chatgptPlatform };
}
