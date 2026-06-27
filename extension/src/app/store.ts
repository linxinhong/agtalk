import { create } from 'zustand';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';
import type { AgtalkConfig } from '@/shared/storage/storage';
import type {
  ApiResult,
  Conversation,
  HealthResponse,
  LogItem,
  Message,
  StatusResponse,
} from '@/shared/api/types';

export type AppPage = 'conversations' | 'settings' | 'logs' | 'status';

export interface AppState {
  activePage: AppPage;
  health: ApiResult<HealthResponse> | null;
  status: ApiResult<StatusResponse> | null;
  config: AgtalkConfig | null;
  conversations: Conversation[];
  messages: Message[];
  selectedConversationId: string | null;
  selectedMessageId: string | null;
  logs: LogItem[];
  logsError: string | null;
  loading: boolean;
  error: string | null;
  success: string | null;
}

export interface AppActions {
  setActivePage: (page: AppPage) => void;
  setError: (error: string | null) => void;
  setSuccess: (success: string | null) => void;
  clearError: () => void;
  clearSuccess: () => void;
  bootstrap: () => Promise<void>;
  loadHealth: () => Promise<void>;
  loadStatus: () => Promise<void>;
  loadConfig: () => Promise<void>;
  saveConfig: (patch: Partial<AgtalkConfig>) => Promise<boolean>;
  loadConversations: () => Promise<void>;
  selectConversation: (id: string) => Promise<void>;
  selectMessage: (id: string) => void;
  loadMessages: (conversationId: string) => Promise<void>;
  sendReply: (text: string) => Promise<boolean>;
  loadLogs: () => Promise<void>;
  pingBackground: () => Promise<boolean>;
}

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

function isOffline(): boolean {
  if (typeof navigator === 'undefined') return false;
  return !navigator.onLine;
}

export const useAppStore = create<AppState & AppActions>()((set, get) => ({
  activePage: 'conversations',
  health: null,
  status: null,
  config: null,
  conversations: [],
  messages: [],
  selectedConversationId: null,
  selectedMessageId: null,
  logs: [],
  logsError: null,
  loading: false,
  error: null,
  success: null,

  setActivePage: (page) => set({ activePage: page }),
  setError: (error) => set({ error }),
  setSuccess: (success) => set({ success }),
  clearError: () => set({ error: null }),
  clearSuccess: () => set({ success: null }),

  bootstrap: async () => {
    set({ loading: true, error: null });
    await Promise.allSettled([
      get().loadHealth(),
      get().loadStatus(),
      get().loadConfig(),
    ]);
    set({ loading: false });
    await get().loadConversations();
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

  loadConfig: async () => {
    try {
      const res = await sendMessage<unknown, { ok: true; data: AgtalkConfig } | { ok: false; error: unknown }>({
        type: MessageType.API_GET_CONFIG,
      });
      if (res?.ok) {
        set({ config: res.data });
      } else {
        set({ error: parseError(res) });
      }
    } catch (err) {
      set({ error: parseError(err) });
    }
  },

  saveConfig: async (patch) => {
    set({ loading: true, error: null, success: null });
    try {
      const res = await sendMessage<Partial<AgtalkConfig>, { ok: true; data: AgtalkConfig } | { ok: false; error: unknown }>({
        type: MessageType.API_SAVE_CONFIG,
        payload: patch,
      });
      if (res?.ok) {
        set({ config: res.data, success: '设置已保存' });
        await get().loadStatus();
        return true;
      }
      set({ error: parseError(res) });
      return false;
    } catch (err) {
      set({ error: parseError(err) });
      return false;
    } finally {
      set({ loading: false });
    }
  },

  loadConversations: async () => {
    if (isOffline()) return;
    try {
      const res = await sendMessage<unknown, ApiResult<Conversation[]>>({
        type: MessageType.API_GET_CONVERSATIONS,
      });
      if (res?.ok) {
        const list = Array.isArray(res.data) ? res.data : [];
        set({ conversations: list });
        if (list.length > 0 && !get().selectedConversationId) {
          await get().selectConversation(list[0].id);
        }
      } else {
        set({ error: parseError(res) });
      }
    } catch (err) {
      set({ error: parseError(err) });
    }
  },

  selectConversation: async (id) => {
    set({ selectedConversationId: id });
    await get().loadMessages(id);
  },

  selectMessage: (id) => set({ selectedMessageId: id }),

  loadMessages: async (conversationId) => {
    try {
      const res = await sendMessage<{ id?: string; conversationId?: string }, ApiResult<Message[]>>({
        type: MessageType.API_GET_CONVERSATION_MESSAGES,
        payload: { id: conversationId },
      });
      if (res?.ok) {
        const list = Array.isArray(res.data) ? res.data : [];
        set({ messages: list });
        const lastMsg = list[list.length - 1];
        set({ selectedMessageId: lastMsg?.id || null });
      } else {
        set({ messages: [], selectedMessageId: null, error: parseError(res) });
      }
    } catch (err) {
      set({ messages: [], selectedMessageId: null, error: parseError(err) });
    }
  },

  sendReply: async (text) => {
    const { selectedMessageId, selectedConversationId } = get();
    const replyToMsgId = selectedMessageId;
    if (!replyToMsgId) {
      set({ error: '没有可回复的消息' });
      return false;
    }
    if (!text.trim()) {
      set({ error: '回复内容为空' });
      return false;
    }
    set({ loading: true, error: null, success: null });
    try {
      const res = await sendMessage<{ id?: string; replyToMsgId?: string; text?: string }, { ok: true; data?: unknown } | { ok: false; error: unknown }>({
        type: MessageType.API_SEND_REPLY,
        payload: { id: replyToMsgId, text: text.trim() },
      });
      if (res?.ok) {
        set({ success: '回复已发送' });
        if (selectedConversationId) {
          await get().loadMessages(selectedConversationId);
        }
        await get().loadConversations();
        return true;
      }
      set({ error: parseError(res) });
      return false;
    } catch (err) {
      set({ error: parseError(err) });
      return false;
    } finally {
      set({ loading: false });
    }
  },

  loadLogs: async () => {
    set({ logs: [], logsError: null });
    try {
      const res = await sendMessage<unknown, ApiResult<LogItem[]>>({
        type: MessageType.API_GET_LOGS,
      });
      if (res?.ok) {
        set({ logs: Array.isArray(res.data) ? res.data : [] });
      } else {
        set({ logsError: parseError(res) });
      }
    } catch (err) {
      set({ logsError: parseError(err) });
    }
  },

  pingBackground: async () => {
    try {
      const res = await sendMessage<unknown, { pong?: boolean }>({ type: MessageType.PING_BACKGROUND });
      return res?.pong ?? false;
    } catch {
      return false;
    }
  },
}));
