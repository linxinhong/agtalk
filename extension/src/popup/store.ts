import { create } from 'zustand';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';
import type { AgtalkConfig } from '@/shared/storage/storage';
import type { ApiResult, HealthResponse, StatusResponse, InboxItem, Peer } from '@/shared/api/types';

export type PopupPage =
  | 'home'
  | 'agents'
  | 'agentConfig'
  | 'localService'
  | 'platformConfig'
  | 'debug';

export interface PeerListData {
  peers: Array<Peer & { connected: boolean; autoInject: boolean }>;
  connectedPeers: string[];
  autoInjectPeers: string[];
  activePeer: string;
  agentName: string;
}

interface InboxSummary {
  items: InboxItem[];
  unread: number;
  total: number;
  migrationPending?: boolean;
  error?: string;
}

export interface PopupState {
  page: PopupPage;
  pageStack: PopupPage[];
  config: AgtalkConfig | null;
  health: ApiResult<HealthResponse> | null;
  status: ApiResult<StatusResponse> | null;
  peers: PeerListData | null;
  inbox: InboxSummary | null;
  loading: boolean;
  lastError: string | null;
}

export interface PopupActions {
  navigate: (page: PopupPage) => void;
  back: () => void;
  loadConfig: () => Promise<void>;
  loadHealth: () => Promise<void>;
  loadStatus: () => Promise<void>;
  loadPeers: () => Promise<void>;
  loadInbox: () => Promise<void>;
  loadAll: () => Promise<void>;
  saveConfig: (patch: Partial<AgtalkConfig>) => Promise<AgtalkConfig | null>;
  setAutoInject: (enabled: boolean) => Promise<void>;
  connectPeer: (name: string) => Promise<void>;
  disconnectPeer: (name: string) => Promise<void>;
  togglePeerAutoInject: (name: string) => Promise<void>;
  setActivePeer: (name: string) => Promise<void>;
  registerAgent: () => Promise<boolean>;
  reconnect: () => Promise<boolean>;
  setLastError: (error: string | null) => void;
}

const initialPage: PopupPage = 'home';

function parseError(res: unknown): string {
  if (!res) return '未知错误';
  if (typeof res === 'string') return res;
  if (typeof res === 'object' && res !== null) {
    const r = res as { ok?: boolean; error?: unknown };
    if (r.ok === false) {
      const err = r.error;
      if (typeof err === 'string') return err;
      if (typeof err === 'object' && err !== null) {
        const e = err as { code?: string; message?: string };
        return `${e.code || 'error'}: ${e.message || '未知错误'}`;
      }
    }
  }
  return '未知错误';
}

export const usePopupStore = create<PopupState & PopupActions>((set, get) => ({
  page: initialPage,
  pageStack: [],
  config: null,
  health: null,
  status: null,
  peers: null,
  inbox: null,
  loading: false,
  lastError: null,

  navigate: (page) => {
    const { pageStack, page: current } = get();
    set({ page, pageStack: [...pageStack, current] });
  },

  back: () => {
    const { pageStack } = get();
    if (pageStack.length === 0) {
      set({ page: 'home' });
      return;
    }
    const prev = pageStack[pageStack.length - 1];
    set({ page: prev, pageStack: pageStack.slice(0, -1) });
  },

  loadConfig: async () => {
    try {
      const res = await sendMessage<unknown, { ok: true; data: AgtalkConfig } | { ok: false; error: unknown }>({
        type: MessageType.GET_CONFIG,
      });
      if (res?.ok) {
        set({ config: res.data });
      } else {
        set({ lastError: parseError(res) });
      }
    } catch (err) {
      set({ lastError: parseError(err) });
    }
  },

  loadHealth: async () => {
    try {
      const res = await sendMessage<unknown, ApiResult<HealthResponse>>({
        type: MessageType.API_HEALTH_CHECK,
      });
      set({ health: res ?? null });
    } catch (err) {
      set({ health: { ok: false, error: { code: 'send_error', message: parseError(err) } } });
    }
  },

  loadStatus: async () => {
    try {
      const res = await sendMessage<unknown, ApiResult<StatusResponse>>({
        type: MessageType.API_GET_STATUS,
      });
      set({ status: res ?? null });
    } catch (err) {
      set({ status: { ok: false, error: { code: 'send_error', message: parseError(err) } } });
    }
  },

  loadPeers: async () => {
    try {
      const res = await sendMessage<unknown, { ok: true; data: PeerListData } | { ok: false; error: unknown }>({
        type: MessageType.GET_CONNECTED_PEERS,
      });
      if (res?.ok) {
        set({ peers: res.data });
      } else {
        set({ lastError: parseError(res) });
      }
    } catch (err) {
      set({ lastError: parseError(err) });
    }
  },

  loadInbox: async () => {
    try {
      const inboxRes = await sendMessage<{ limit?: number }, ApiResult<InboxItem[]>>({
        type: MessageType.AGTALK_INBOX,
        payload: { limit: 5 },
      });
      if (inboxRes?.ok) {
        const raw = inboxRes.data;
        const items = Array.isArray(raw)
          ? raw
          : (raw as unknown as { items?: InboxItem[]; messages?: InboxItem[] }).items ??
            (raw as unknown as { items?: InboxItem[]; messages?: InboxItem[] }).messages ??
            [];
        const normalized = items.slice(0, 5).map((i) => ({
          ...i,
          from_name: i.from_name || (i as any).from?.name || (i as any).sender_name || '未知',
          body: i.body || (i as any).content?.body || '',
        }));
        const unread = normalized.filter((i) => {
          const delivery = i.delivery || (i as any).recipients?.[0];
          return !i.read_at && !delivery?.read_at && (i.status === 'pending' || i.status === 'unread');
        }).length;
        set({ inbox: { items: normalized, unread, total: normalized.length } });
        return;
      }

      const statsRes = await sendMessage<unknown, { ok: true; data: { unread: number; total: number } } | { ok: false; error: unknown }>({
        type: MessageType.AGTALK_INBOX_STATS,
      });
      if (statsRes?.ok) {
        set({ inbox: { items: [], unread: statsRes.data.unread, total: statsRes.data.total } });
        return;
      }

      set({ inbox: { items: [], unread: 0, total: 0, migrationPending: true, error: parseError(inboxRes) } });
    } catch (err) {
      set({ inbox: { items: [], unread: 0, total: 0, migrationPending: true, error: parseError(err) } });
    }
  },

  loadAll: async () => {
    set({ loading: true, lastError: null });
    await Promise.allSettled([
      get().loadConfig(),
      get().loadHealth(),
      get().loadStatus(),
      get().loadPeers(),
      get().loadInbox(),
    ]);
    set({ loading: false });
  },

  saveConfig: async (patch) => {
    const res = await sendMessage<Partial<AgtalkConfig>, { ok: true; data: AgtalkConfig } | { ok: false; error: unknown }>({
      type: MessageType.SAVE_CONFIG,
      payload: patch,
    });
    if (res?.ok) {
      set({ config: res.data, lastError: null });
      return res.data;
    }
    set({ lastError: parseError(res) });
    return null;
  },

  setAutoInject: async (enabled) => {
    const next: Partial<AgtalkConfig> = { autoInject: enabled };
    if (!enabled) {
      next.autoInjectPeers = [];
      // content script 自动回复逻辑未迁移，但保留消息契约
      await sendMessage({ type: MessageType.PAUSE_ALL_AUTO_REPLY });
    }
    const saved = await get().saveConfig(next);
    if (saved) {
      await Promise.all([get().loadPeers(), get().loadStatus()]);
    }
  },

  connectPeer: async (name) => {
    const config = get().config;
    const connected = new Set(config?.connectedPeers || []);
    connected.add(name);
    const saved = await get().saveConfig({
      connectedPeers: Array.from(connected),
      activePeer: name,
      targetAgent: name,
    });
    if (saved) {
      await get().loadPeers();
    }
  },

  disconnectPeer: async (name) => {
    const config = get().config;
    const connected = (config?.connectedPeers || []).filter((p) => p !== name);
    const autoInject = (config?.autoInjectPeers || []).filter((p) => p !== name);
    const fallback = connected[0] || '';
    const activePeer = config?.activePeer === name ? fallback : config?.activePeer || '';
    const targetAgent = config?.targetAgent === name ? fallback : config?.targetAgent || '';
    const saved = await get().saveConfig({
      connectedPeers: connected,
      autoInjectPeers: autoInject,
      activePeer,
      targetAgent,
    });
    if (saved) {
      await get().loadPeers();
    }
  },

  togglePeerAutoInject: async (name) => {
    const config = get().config;
    if (!config?.autoInject) return;
    const autoInject = new Set(config.autoInjectPeers || []);
    if (autoInject.has(name)) {
      autoInject.delete(name);
    } else {
      autoInject.add(name);
    }
    const saved = await get().saveConfig({ autoInjectPeers: Array.from(autoInject) });
    if (saved) {
      await get().loadPeers();
    }
  },

  setActivePeer: async (name) => {
    const saved = await get().saveConfig({ activePeer: name, targetAgent: name });
    if (saved) {
      await get().loadPeers();
    }
  },

  registerAgent: async () => {
    set({ loading: true, lastError: null });
    const res = await sendMessage<unknown, { ok: true; data: { session_id: string; participant: string } } | { ok: false; error: unknown }>({
      type: MessageType.REGISTER_AGENT,
    });
    set({ loading: false });
    if (res?.ok) {
      await get().loadAll();
      return true;
    }
    set({ lastError: parseError(res) });
    return false;
  },

  reconnect: async () => {
    set({ loading: true, lastError: null });
    const res = await sendMessage<unknown, { ok: true; data: { session_id: string; participant: string } } | { ok: false; error: unknown }>({
      type: MessageType.RECONNECT,
    });
    set({ loading: false });
    if (res?.ok) {
      await get().loadAll();
      return true;
    }
    set({ lastError: parseError(res) });
    return false;
  },

  setLastError: (error) => set({ lastError: error }),
}));
