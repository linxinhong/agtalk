// 智谱清言 (chatglm.cn) 平台适配
const chatglmPlatform = {
  id: 'chatglm',
  name: 'ChatGLM',
  match: ['https://chatglm.cn/*'],

  selectors: {
    // 对话容器：新版 ChatGLM 主内容区
    chatContainer: '.main-chat-content, .chat-content, main, .chat-container',
    // 消息节点：答案块 + 用户问题块
    messageItems: '.answer-content, .question-content, .message-item, .chat-message',
    userMessageAttr: null,
    aiMessageAttr: null,
    messageText: '.markdown-body',
    // 输入框：优先 contenteditable，再降级 textarea
    inputBox: '[contenteditable="true"], .input-wrap textarea, .input-box textarea, .input-area textarea, textarea[placeholder*="输入"], textarea.gm-input',
    stopButton: null,
  },

  isUserMessage(node) {
    return node.classList.contains('question-content') ||
      node.classList.contains('user') ||
      !!node.querySelector('.user-message, [class*="user"]');
  },

  isAiMessage(node) {
    return node.classList.contains('answer-content') ||
      !!node.querySelector('.answer-content-wrap, .markdown-body');
  },

  extractText(node, isUser) {
    if (isUser) {
      const area = node.querySelector('.question-content, .user-message, .markdown-body');
      return area ? area.innerText.trim() : node.innerText.trim();
    }

    // AI 答案可能同时包含「思考过程」和「最终答案」，只取最终答案
    const wraps = node.querySelectorAll('.answer-content-wrap');
    for (let i = wraps.length - 1; i >= 0; i--) {
      if (wraps[i].closest('.advance-thinking')) continue;
      const md = wraps[i].querySelector('.markdown-body');
      if (md) return md.innerText.trim();
    }

    // 兜底：排除思考区域里的 markdown-body
    const md = node.querySelector('.markdown-body:not(.advance-thinking .markdown-body)');
    if (md) return md.innerText.trim();

    return node.innerText.trim();
  },

  injectText(text) {
    return new Promise((resolve) => {
      const input = document.querySelector(this.selectors.inputBox);
      if (!input) {
        console.warn('[ChatGLM] 未找到输入框');
        resolve({ success: false, error: '未找到输入框' });
        return;
      }

      const isContentEditable = input.isContentEditable;
      input.focus();
      input.click();

      if (isContentEditable) {
        input.textContent = text;
      } else {
        const descriptor = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value');
        if (descriptor && descriptor.set) {
          descriptor.set.call(input, text);
        } else {
          input.value = text;
        }
      }

      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
      input.dispatchEvent(new Event('keyup', { bubbles: true }));

      setTimeout(() => {
        const sendBtn = document.querySelector(this.selectors.sendButton);
        if (sendBtn && sendBtn.offsetParent !== null) {
          sendBtn.click();
          console.log('[ChatGLM] 已点击发送按钮');
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
          console.log('[ChatGLM] 已回车发送');
          resolve({ success: true, method: 'enter' });
        }
      }, 300);
    });
  },

  isStopButtonPresent() {
    return false;
  },
};

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { chatglmPlatform };
}
