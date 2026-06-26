import { defineContentScript } from 'wxt/sandbox';
import { MessageType } from '@/shared/messaging/message-types';

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
    chrome.runtime.onMessage.addListener((
      message: { type?: string },
      _sender: chrome.runtime.MessageSender,
      sendResponse: (response: unknown) => void
    ) => {
      if (message?.type === MessageType.PING) {
        sendResponse({ pong: true, source: 'wxt-content' });
        return true;
      }
      return false;
    });
  },
});
