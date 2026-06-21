// IndexedDB 本地消息存储
const DB_NAME = 'agtalk_bridge';
const DB_VERSION = 1;
const STORE_NAME = 'messages';

function openDB() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
    request.onupgradeneeded = (event) => {
      const db = event.target.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        const store = db.createObjectStore(STORE_NAME, { keyPath: 'id' });
        store.createIndex('created_at', 'created_at', { unique: false });
        store.createIndex('from_name', 'from_name', { unique: false });
        store.createIndex('status', 'status', { unique: false });
        store.createIndex('saved_at', 'saved_at', { unique: false });
      }
    };
  });
}

async function getStore(mode = 'readonly') {
  const db = await openDB();
  return db.transaction(STORE_NAME, mode).objectStore(STORE_NAME);
}

function normalizeMessage(item) {
  const delivery = item.delivery || (item.recipients?.[0] ? {
    status: item.recipients[0].status,
    read_at: item.recipients[0].read_at,
    done_at: item.recipients[0].done_at,
  } : {});
  return {
    id: item.id,
    chat_id: item.chat_id || null,
    from_id: item.from?.id || item.from_agent || '',
    from_name: item.from?.name || item.from_agent || '未知',
    from_type: item.from?.type || '',
    to_name: item.recipients?.map((r) => r.recipient_name).join(', ') || '',
    subject: item.subject || null,
    body: item.content?.body || item.body || '',
    content_type: item.content_type || 'text',
    status: delivery.status || item.status || 'pending',
    read_at: delivery.read_at || null,
    done_at: delivery.done_at || null,
    created_at: item.created_at || null,
    reply_to_id: item.reply_to_id || null,
    injected: false,
    saved_at: Date.now(),
  };
}

const MessageStore = {
  async save(item) {
    const store = await getStore('readwrite');
    const msg = normalizeMessage(item);
    return new Promise((resolve, reject) => {
      const request = store.put(msg);
      request.onsuccess = () => resolve({ ok: true });
      request.onerror = () => reject(request.error);
    });
  },

  async saveMany(items) {
    const store = await getStore('readwrite');
    const promises = items.map((item) => new Promise((resolve, reject) => {
      const request = store.put(normalizeMessage(item));
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    }));
    await Promise.all(promises);
    return { ok: true, count: items.length };
  },

  async getRecent(limit = 100) {
    const store = await getStore('readonly');
    const index = store.index('saved_at');
    return new Promise((resolve, reject) => {
      const request = index.openCursor(null, 'prev');
      const results = [];
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (!cursor || results.length >= limit) {
          resolve(results);
          return;
        }
        results.push(cursor.value);
        cursor.continue();
      };
      request.onerror = () => reject(request.error);
    });
  },

  async getById(id) {
    const store = await getStore('readonly');
    return new Promise((resolve, reject) => {
      const request = store.get(id);
      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => reject(request.error);
    });
  },

  async markInjected(id, injected = true) {
    const store = await getStore('readwrite');
    return new Promise((resolve, reject) => {
      const getReq = store.get(id);
      getReq.onsuccess = () => {
        const msg = getReq.result;
        if (!msg) {
          resolve({ ok: false, error: '消息不存在' });
          return;
        }
        msg.injected = injected;
        msg.saved_at = Date.now();
        const putReq = store.put(msg);
        putReq.onsuccess = () => resolve({ ok: true });
        putReq.onerror = () => reject(putReq.error);
      };
      getReq.onerror = () => reject(getReq.error);
    });
  },

  async search(query, limit = 50) {
    const store = await getStore('readonly');
    return new Promise((resolve, reject) => {
      const request = store.openCursor();
      const results = [];
      const q = query.toLowerCase();
      request.onsuccess = (event) => {
        const cursor = event.target.result;
        if (!cursor || results.length >= limit) {
          resolve(results);
          return;
        }
        const msg = cursor.value;
        if ((msg.body && msg.body.toLowerCase().includes(q)) ||
            (msg.from_name && msg.from_name.toLowerCase().includes(q)) ||
            (msg.subject && msg.subject.toLowerCase().includes(q))) {
          results.push(msg);
        }
        cursor.continue();
      };
      request.onerror = () => reject(request.error);
    });
  },

  async clear() {
    const store = await getStore('readwrite');
    return new Promise((resolve, reject) => {
      const request = store.clear();
      request.onsuccess = () => resolve({ ok: true });
      request.onerror = () => reject(request.error);
    });
  },
};

export { MessageStore };

// CommonJS 兼容（Node 测试用）
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { MessageStore };
}
