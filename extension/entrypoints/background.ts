export default defineBackground(() => {
  console.log('[WXT BG] agtalk service worker started');

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    switch (message?.type) {
      case 'PING_BACKGROUND': {
        sendResponse({ ok: true, pong: true, source: 'wxt-background' });
        return true;
      }
      case 'OPEN_APP_PAGE': {
        try {
          chrome.tabs.create({ url: chrome.runtime.getURL('/app.html') }, () => {
            if (chrome.runtime.lastError) {
              sendResponse({ ok: false, error: chrome.runtime.lastError.message });
            } else {
              sendResponse({ ok: true });
            }
          });
        } catch (err: any) {
          sendResponse({ ok: false, error: err?.message || '打开页面失败' });
        }
        return true;
      }
      default:
        return false;
    }
  });
});
