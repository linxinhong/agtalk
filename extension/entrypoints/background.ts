export default defineBackground(() => {
  console.log('[WXT BG] agtalk service worker started');

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (message?.type === 'WXT_PING') {
      sendResponse({ ok: true, pong: true });
      return true;
    }
    return false;
  });
});
