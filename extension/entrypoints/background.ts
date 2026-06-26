import { defineBackground } from 'wxt/sandbox';
import { MessageType } from '@/shared/messaging/message-types';
import { createBackgroundRouter } from '@/shared/messaging/handlers';
import { getHealth, getStatus } from '@/shared/api/client';

export default defineBackground(() => {
  console.log('[WXT BG] agtalk service worker started');

  const router = createBackgroundRouter({
    [MessageType.PING_BACKGROUND]: async () => ({
      ok: true,
      pong: true,
      source: 'wxt-background',
    }),

    [MessageType.OPEN_APP_PAGE]: async () => {
      try {
        await chrome.tabs.create({ url: chrome.runtime.getURL('/app.html') });
        return { ok: true };
      } catch (err) {
        return { ok: false, error: err instanceof Error ? err.message : '打开页面失败' };
      }
    },

    [MessageType.API_HEALTH_CHECK]: async () => {
      const result = await getHealth();
      return result;
    },

    [MessageType.API_GET_STATUS]: async () => {
      const result = await getStatus();
      return result;
    },
  });

  chrome.runtime.onMessage.addListener(router);
});
