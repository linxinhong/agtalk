import type { JoinResult, Mailbox, Message, StoredSession } from './types';

export const DEFAULT_BASE_URL = 'http://127.0.0.1:19527';
export const STORAGE_KEY_BASE_URL = 'baseUrl';

export interface SendOptions {
  subject?: string;
  files?: string[];
  notify?: boolean;
  more?: boolean;
}

/** 读取用户配置的 daemon 地址，未配置则回退到默认本地地址。 */
export async function getBaseUrl(): Promise<string> {
  const { [STORAGE_KEY_BASE_URL]: stored } = await browser.storage.local.get(STORAGE_KEY_BASE_URL);
  if (typeof stored === 'string' && stored.trim()) {
    return stored.trim().replace(/\/+$/, '');
  }
  return DEFAULT_BASE_URL;
}

/** 持久化用户配置的 daemon 地址。 */
export async function setBaseUrl(url: string): Promise<void> {
  await browser.storage.local.set({ [STORAGE_KEY_BASE_URL]: url.trim().replace(/\/+$/, '') });
}

/** 重置为默认本地地址。 */
export async function resetBaseUrl(): Promise<void> {
  await browser.storage.local.remove(STORAGE_KEY_BASE_URL);
}

export async function join(
  name: string,
  intro: string,
  workspace: string,
): Promise<JoinResult> {
  const base = await getBaseUrl();
  const resp = await fetch(`${base}/api/v1/browser/join`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, intro, workspace }),
  });
  const data = await resp.json();
  if (data.type !== 'browser_join_result') {
    throw new Error(data.message || 'join failed');
  }
  return { address: data.address, name: data.name, token: data.token };
}

export async function leave(token: string, address: string): Promise<void> {
  const base = await getBaseUrl();
  const resp = await fetch(`${base}/api/v1/id/leave`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-AgTalk-Address': address,
      'X-AgTalk-Browser-Token': token,
    },
    body: JSON.stringify({}),
  });
  if (!resp.ok) {
    const data = await resp.json().catch(() => ({}));
    throw new Error(data.message || 'leave failed');
  }
}

export async function lookup(name?: string): Promise<Mailbox[]> {
  const base = await getBaseUrl();
  const url = new URL(`${base}/api/v1/id/lookup`);
  if (name) url.searchParams.set('name', name);
  const resp = await fetch(url.toString());
  const data = await resp.json();
  if (data.type !== 'lookup_result') {
    throw new Error(data.message || 'lookup failed');
  }
  return data.mailboxes as Mailbox[];
}

export async function send(
  token: string,
  address: string,
  to: string,
  body: string,
  opts: SendOptions = {},
): Promise<string> {
  const base = await getBaseUrl();
  const resp = await fetch(`${base}/api/v1/msg/send`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-AgTalk-Address': address,
      'X-AgTalk-Browser-Token': token,
    },
    body: JSON.stringify({
      to,
      body,
      subject: opts.subject,
      files: opts.files ?? [],
      notify: opts.notify,
      more: opts.more ?? false,
    }),
  });
  const data = await resp.json();
  if (data.type !== 'ok') {
    throw new Error(data.message || 'send failed');
  }
  return data.id as string;
}

export interface SubscribeOptions {
  lastEventId?: number;
  onError?: (err: Error) => void;
}

export async function subscribeEvents(
  token: string,
  address: string,
  onMessage: (msg: Message) => void,
  opts: SubscribeOptions = {},
): Promise<() => void> {
  const base = await getBaseUrl();
  const controller = new AbortController();
  let active = true;
  let lastEventId = opts.lastEventId ?? 0;

  const backoff = [3000, 10000, 30000];
  let attempt = 0;

  async function connect() {
    while (active) {
      try {
        const headers: Record<string, string> = {
          'X-AgTalk-Address': address,
          'X-AgTalk-Browser-Token': token,
        };
        if (lastEventId > 0) {
          headers['Last-Event-ID'] = String(lastEventId);
        }

        const resp = await fetch(`${base}/api/v1/events`, {
          headers,
          signal: controller.signal,
        });

        if (!resp.ok || !resp.body) {
          throw new Error(`events failed: ${resp.status}`);
        }

        attempt = 0;
        const reader = resp.body.getReader();
        let buffer = '';

        while (active) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += new TextDecoder().decode(value, { stream: true });

          let pos: number;
          while ((pos = buffer.indexOf('\n\n')) >= 0) {
            const text = buffer.slice(0, pos);
            buffer = buffer.slice(pos + 2);
            const evt = parseEvent(text);
            if (evt.id) {
              const id = Number(evt.id);
              if (!Number.isNaN(id)) lastEventId = id;
            }
            if (evt.data) {
              try {
                const msg = JSON.parse(evt.data) as Message;
                onMessage(msg);
              } catch {
                // ignore malformed data
              }
            }
          }
        }
      } catch (err) {
        if (!active) break;
        if (opts.onError && err instanceof Error) opts.onError(err);
      }

      const delay = backoff[Math.min(attempt, backoff.length - 1)];
      attempt += 1;
      await sleep(delay);
    }
  }

  connect();

  return () => {
    active = false;
    controller.abort();
  };
}

function parseEvent(text: string): { id?: string; event?: string; data?: string } {
  const evt: { id?: string; event?: string; data?: string } = {};
  let data = '';
  for (const line of text.split('\n')) {
    if (line.startsWith('id:')) evt.id = line.slice(3).trim();
    else if (line.startsWith('event:')) evt.event = line.slice(6).trim();
    else if (line.startsWith('data:')) {
      if (data) data += '\n';
      data += line.slice(5).trimStart();
    }
  }
  if (data) evt.data = data;
  return evt;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
