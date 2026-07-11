import {
  join,
  leave,
  send,
  subscribeEvents,
  getBaseUrl,
  setBaseUrl,
  resetBaseUrl,
  DEFAULT_BASE_URL,
} from '@/api';
import type { Message, StoredSession } from '@/types';

export default defineBackground(() => {
  let unsubscribe: (() => void) | null = null;

  async function loadSession(): Promise<StoredSession | null> {
    const stored = await browser.storage.local.get(['address', 'token', 'name']);
    if (stored.address && stored.token) {
      return {
        address: String(stored.address),
        token: String(stored.token),
        name: stored.name ? String(stored.name) : undefined,
      };
    }
    return null;
  }

  async function saveSession(session: StoredSession | null) {
    if (session) {
      await browser.storage.local.set({
        address: session.address,
        token: session.token,
        name: session.name ?? '',
      });
    } else {
      await browser.storage.local.remove(['address', 'token', 'name']);
    }
  }

  async function addMessage(msg: Message) {
    const { messages = [] } = await browser.storage.local.get('messages');
    const list = (messages as Message[]).slice(-99);
    list.push(msg);
    await browser.storage.local.set({ messages: list });
    await updateBadge(list.length);
  }

  async function updateBadge(count: number) {
    const text = count > 0 ? String(Math.min(count, 99)) : '';
    await browser.action.setBadgeText({ text });
  }

  async function startSSE() {
    stopSSE();
    const session = await loadSession();
    if (!session) return;

    unsubscribe = await subscribeEvents(
      session.token,
      session.address,
      async (msg) => {
        await addMessage(msg);
      },
      {
        onError: (err) => {
          console.error('[agtalk] SSE error', err);
        },
      },
    );
  }

  function stopSSE() {
    if (unsubscribe) {
      unsubscribe();
      unsubscribe = null;
    }
  }

  browser.runtime.onMessage.addListener(async (request: unknown) => {
    const action = (request as Record<string, unknown>).action as string;

    if (action === 'GET_STATE') {
      const session = await loadSession();
      const { messages = [] } = await browser.storage.local.get('messages');
      const baseUrl = await getBaseUrl();
      return { session, messages, baseUrl, defaultBaseUrl: DEFAULT_BASE_URL };
    }

    if (action === 'SET_BASE_URL') {
      const { url } = request as { url: string };
      await setBaseUrl(url);
      // 地址变了，重连 SSE 才会走新地址
      await startSSE();
      return { ok: true };
    }

    if (action === 'RESET_BASE_URL') {
      await resetBaseUrl();
      await startSSE();
      return { ok: true };
    }

    if (action === 'JOIN') {
      const { name, intro } = request as {
        name: string;
        intro: string;
      };
      const result = await join(name, intro);
      await saveSession({
        address: result.address,
        token: result.token,
        name: result.name,
      });
      await browser.storage.local.set({ messages: [] });
      await updateBadge(0);
      await startSSE();
      return { ok: true, session: result };
    }

    if (action === 'LEAVE') {
      const session = await loadSession();
      if (session) {
        try {
          await leave(session.token, session.address);
        } catch (err) {
          console.error('[agtalk] leave failed', err);
        }
      }
      stopSSE();
      await saveSession(null);
      await browser.storage.local.remove(['messages']);
      await updateBadge(0);
      return { ok: true };
    }

    if (action === 'SEND') {
      const session = await loadSession();
      if (!session) return { ok: false, error: 'not joined' };
      const { to, body } = request as { to: string; body: string };
      const id = await send(session.token, session.address, to, body);
      return { ok: true, id };
    }

    if (action === 'CLEAR_MESSAGES') {
      await browser.storage.local.set({ messages: [] });
      await updateBadge(0);
      return { ok: true };
    }

    return { ok: false, error: 'unknown action' };
  });

  startSSE();
});
