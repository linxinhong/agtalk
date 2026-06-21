// Sider.ai 平台适配（参考 sider-talk）
const siderPlatform = {
  id: 'sider',
  name: 'Sider',
  match: ['https://sider.ai/*'],

  selectors: {
    chatContainer: '.chat-list',
    messageItems: '.message-item',
    userMessageAttr: null,
    aiMessageAttr: '.content-area',
    messageText: '.message-inner',
    aiContentArea: '.content-area',
    inputBox: 'textarea[class*="chatBox-input"]',
    sendButton: '.send-btn:not([disabled]), button[type="submit"]:not([disabled]), [data-testid="send-button"]:not([disabled]), button[class*="send"]:not([disabled])',
    stopButton: null,
    stopButtonText: '停止生成',
  },

  isUserMessage(node) {
    return !node.querySelector(this.selectors.aiContentArea);
  },

  isAiMessage(node) {
    return !!node.querySelector(this.selectors.aiContentArea);
  },

  extractText(node, isUser) {
    if (isUser) {
      const inner = node.querySelector(this.selectors.messageText);
      return inner ? inner.innerText.trim() : node.innerText.trim();
    }
    const area = node.querySelector(this.selectors.aiContentArea);
    return area ? area.innerText.trim() : node.innerText.trim();
  },

  injectText(text) {
    const input = document.querySelector(this.selectors.inputBox);
    if (!input) return { success: false, error: '未找到输入框' };

    input.focus();
    input.click();

    // 兼容 React 受控组件：使用原型 setter 触发内部状态更新
    const descriptor = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
    if (descriptor && descriptor.set) {
      descriptor.set.call(input, text);
    } else {
      input.value = text;
    }

    // 触发必要事件让 Sider 识别输入变化
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
    const text = this.selectors.stopButtonText;
    const buttons = Array.from(document.querySelectorAll('button, span'));
    return buttons.some((btn) =>
      btn.textContent.trim() === text && btn.offsetParent !== null
    );
  },
};

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { siderPlatform };
}
