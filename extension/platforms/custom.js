// 用户自定义平台 —— 从 chrome.storage.local 加载配置
let customPlatform = null;

async function loadCustomPlatform() {
  const result = await chrome.storage.local.get('agtalk_custom_platform');
  const cfg = result.agtalk_custom_platform;
  if (!cfg || !cfg.match || !cfg.selectors) {
    customPlatform = null;
    return;
  }

  customPlatform = {
    id: 'custom',
    name: cfg.name || 'Custom',
    match: Array.isArray(cfg.match) ? cfg.match : [cfg.match],
    selectors: cfg.selectors,

    isUserMessage(node) {
      if (cfg.isUserMessage?.attr) {
        return node.getAttribute(cfg.isUserMessage.attr) === cfg.isUserMessage.value;
      }
      if (cfg.isUserMessage?.selector) {
        return !!node.querySelector(cfg.isUserMessage.selector);
      }
      return false;
    },

    isAiMessage(node) {
      if (cfg.isAiMessage?.attr) {
        return node.getAttribute(cfg.isAiMessage.attr) === cfg.isAiMessage.value;
      }
      if (cfg.isAiMessage?.selector) {
        return !!node.querySelector(cfg.isAiMessage.selector);
      }
      return !this.isUserMessage(node);
    },

    extractText(node, isUser) {
      const selector = isUser
        ? cfg.selectors.userMessageText || cfg.selectors.messageText
        : cfg.selectors.messageText;
      const area = selector ? node.querySelector(selector) : null;
      return area ? area.innerText.trim() : node.innerText.trim();
    },

    async injectText(text) {
      const input = document.querySelector(cfg.selectors.inputBox);
      if (!input) return { success: false, error: '未找到输入框' };

      input.focus();
      if (input.isContentEditable) {
        input.innerText = text;
      } else {
        input.value = text;
      }
      input.dispatchEvent(new Event('input', { bubbles: true }));

      return new Promise((resolve) => {
        setTimeout(() => {
          const sendBtn = cfg.selectors.sendButton
            ? document.querySelector(cfg.selectors.sendButton)
            : null;
          if (sendBtn && !sendBtn.disabled && sendBtn.offsetParent !== null) {
            sendBtn.click();
            resolve({ success: true, method: 'click' });
          } else {
            input.dispatchEvent(new KeyboardEvent('keydown', {
              key: 'Enter', code: 'Enter', keyCode: 13, which: 13,
              bubbles: true, cancelable: true,
            }));
            resolve({ success: true, method: 'enter' });
          }
        }, 200);
      });
    },

    isStopButtonPresent() {
      if (!cfg.selectors.stopButton) return false;
      const btn = document.querySelector(cfg.selectors.stopButton);
      return !!(btn && btn.offsetParent !== null);
    },
  };
}

if (typeof module !== 'undefined' && module.exports) {
  module.exports = { loadCustomPlatform, getCustomPlatform: () => customPlatform };
}
