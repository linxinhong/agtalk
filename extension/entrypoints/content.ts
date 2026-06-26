import { defineContentScript } from 'wxt/sandbox';

export default defineContentScript({
  matches: [
    'https://chatgpt.com/*',
    'https://claude.ai/*',
    'https://sider.ai/*',
    'https://chatglm.cn/*',
  ],
  runAt: 'document_idle',
  main() {
    console.log('[WXT CS] agtalk content script injected on', window.location.href);
    chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
      if (message?.type === 'PING') {
        sendResponse({ pong: true, source: 'wxt-content' });
        return true;
      }
      return false;
    });
  },
});
