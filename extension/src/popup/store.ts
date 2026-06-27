import { create } from 'zustand';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';
import type { AgtalkConfig } from '@/shared/storage/storage';
import type { ApiResult, HealthResponse, StatusResponse } from '@/shared/api/types';
import type { Peer } from '@/shared/api/types';

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

interface PopupState {
  page: PopupPage;
  pageStack: PopupPage[];
  config: AgtalkConfig | null;
  status: ApiResult<StatusResponse> | null;
  health: ApiResult<HealthResponse> | null;
  peers: PeerListData | null;
  loading: boolean;
  lastError: string | null;
}

interface PopupActions {
  navigate: (page: PopupPage) => void;
  back: () => void;
  loadConfig: () => Promise<void>;
  loadStatus: () => Promise<void>;
  loadHealth: () => Promise<void>;
  loadPeers: () => Promise<void>;
  loadAll: () => Promise<void>;
  saveConfig: (patch: Partial<AgtalkConfig>) => Promise<AgtalkConfig | null>;
  setAutoInject: (enabled: boolean) => Promise<void>;
  connectPeer: (name: string) => Promise<void>;
  disconnectPeer: (name: string) => Promise<void>;
  togglePeerAutoInject: (name: string) => Promise<void>;
  registerAgent: () => Promise<boolean>;
  reconnect: () => Promise<boolean>;
  setLastError: (error: string | null) => void;
}

const initialPage: PopupPage = 'home';

function parseError(res: { ok: false; error: { code: string; message: string } } | null): string {
  if (!res) return '未知错误';
  return `${res.error.code}: ${res.error.message}`;
}

export const usePopupStore = create<PopupState & PopupActions>((set, get) => ({
  page: initialPage,
  pageStack: [],
  config: null,
  status: null,
  health: null,
  peers: null,
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
    const res = await sendMessage<unknown, { ok: true; data: AgtalkConfig } | { ok: false; error: { code: string; message: string } }>({
      type: MessageType.GET_CONFIG,
    });
    if (res?.ok) {
      set({ config: res.data, lastError: null });
    } else {
      set({ lastError: parseError(res || null) });
    }
  },

  loadStatus: async () => {
    const res = await sendMessage<unknown, ApiResult<StatusResponse>>({
      type: MessageType.API_GET_STATUS,
    });
    set({ status: res ?? null });
  },

  loadHealth: async () => {
    const res = await sendMessage<unknown, ApiResult<HealthResponse>>({
      type: MessageType.API_HEALTH_CHECK,
    });
    set({ health: res ?? null });
  },

  loadPeers: async () => {
    const res = await sendMessage<unknown, { ok: true; data: PeerListData } | { ok: false; error: { code: string; message: string } }>({
      type: MessageType.GET_CONNECTED_PEERS,
    });
    if (res?.ok) {
      set({ peers: res.data, lastError: null });
    } else {
      set({ lastError: parseError(res || null) });
    }
  },

  loadAll: async () => {
    set({ loading: true, lastError: null });
    await Promise.all([
      get().loadConfig(),
      get().loadStatus(),
      get().loadPeers(),
    ]);
    set({ loading: false });
  },

  saveConfig: async (patch) => {
    const res = await sendMessage<Partial<AgtalkConfig>, { ok: true; data: AgtalkConfig } | { ok: false; error: { code: string; message: string } }>({
      type: MessageType.SAVE_CONFIG,
      payload: patch,
    });
    if (res?.ok) {
      set({ config: res.data, lastError: null });
      return res.data;
    }
    set({ lastError: parseError(res || null) });
    return null;
  },

  setAutoInject: async (enabled) => {
    const next: Partial<AgtalkConfig> = { autoInject: enabled };
    if (!enabled) {
      next.autoInjectPeers = [];
    }
    const saved = await get().saveConfig(next);
    if (saved) {
      await get().loadPeers();
    }
  },

  connectPeer: async (name) => {
    const config = get().config;
    const connected = new Set(config?.connectedPeers || []);
    connected.add(name);
    const saved = await get().saveConfig({ connectedPeers: Array.from(connected) });
    if (saved) {
      await get().loadPeers();
    }
  },

  disconnectPeer: async (name) => {
    const config = get().config;
    const connected = (config?.connectedPeers || []).filter((p) => p !== name);
    const autoInject = (config?.autoInjectPeers || []).filter((p) => p !== name);
    const saved = await get().saveConfig({ connectedPeers: connected, autoInjectPeers: autoInject });
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

  registerAgent: async () => {
    set({ loading: true, lastError: null });
    const res = await sendMessage<unknown, { ok: true; data: { session_id: string; participant: string } } | { ok: false; error: { code: string; message: string } }>({
      type: MessageType.REGISTER_AGENT,
    });
    set({ loading: false });
    if (res?.ok) {
      await get().loadAll();
      return true;
    }
    set({ lastError: parseError(res || null) });
    return false;
  },

  reconnect: async () => {
    set({ loading: true, lastError: null });
    const res = await sendMessage<unknown, { ok: true; data: { session_id: string; participant: string } } | { ok: false; error: { code: string; message: string } }>({
      type: MessageType.RECONNECT,
    });
    set({ loading: false });
    if (res?.ok) {
      await get().loadAll();
      return true;
    }
    set({ lastError: parseError(res || null) });
    return false;
  },

  setLastError: (error) => set({ lastError: error }),
}));
