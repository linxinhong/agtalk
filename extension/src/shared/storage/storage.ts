import { StorageKeys } from './keys';

export interface AgtalkConfig {
  daemonUrl?: string;
  agtalkUrl?: string;
  agentName?: string;
  agentRole?: string;
  agentBio?: string;
  agentCapabilities?: string;
  targetAgent?: string;
  activePeer?: string;
  connectedPeers?: string[];
  autoInjectPeers?: string[];
  enabled?: boolean;
  autoForward?: boolean;
  autoReceive?: boolean;
  autoInject?: boolean;
  pollInterval?: number;
  workspaceRoot?: string;
  workspaceName?: string;
  captureDelay?: number;
  enableChatgpt?: boolean;
  enableClaude?: boolean;
  enableSider?: boolean;
  enableChatglm?: boolean;
  enableCustom?: boolean;
  [key: string]: unknown;
}

export interface AgtalkSession {
  session_id: string;
  token: string;
  participant?: string;
  workspace_id?: string;
}

// 兼容旧插件/变体 session 形状，仅在读取时归一化，不修改 storage key
interface LegacySessionShape {
  session?: { id?: string; token?: string };
  name?: string;
  session_id?: string;
  token?: string;
  participant?: string | { name?: string };
  workspace_id?: string;
}

function normalizeParticipant(value: unknown): string | undefined {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object' && 'name' in value && typeof (value as { name?: string }).name === 'string') {
    return (value as { name: string }).name;
  }
  return undefined;
}

export function normalizeSession(raw: LegacySessionShape | null): AgtalkSession | null {
  if (!raw || typeof raw !== 'object') return null;

  const session_id =
    (typeof raw.session_id === 'string' ? raw.session_id : undefined) ||
    (raw.session && typeof raw.session.id === 'string' ? raw.session.id : undefined);
  const token =
    (typeof raw.token === 'string' ? raw.token : undefined) ||
    (raw.session && typeof raw.session.token === 'string' ? raw.session.token : undefined);

  if (!session_id || !token) return null;

  return {
    session_id,
    token,
    participant: normalizeParticipant(raw.participant) || normalizeParticipant(raw.name),
    workspace_id: typeof raw.workspace_id === 'string' ? raw.workspace_id : undefined,
  };
}

const DEFAULT_CONFIG: AgtalkConfig = {
  daemonUrl: 'http://127.0.0.1:19527',
  agtalkUrl: 'http://127.0.0.1:19527',
  agentName: '',
  agentRole: 'web',
  agentBio: 'Web AI bridge participant',
  agentCapabilities: '',
  targetAgent: '',
  activePeer: '',
  connectedPeers: [],
  autoInjectPeers: [],
  enabled: true,
  autoForward: false,
  autoReceive: true,
  autoInject: false,
  pollInterval: 5000,
  workspaceRoot: '/virtual/web-bridge',
  workspaceName: 'web-bridge',
  captureDelay: 300,
  enableChatgpt: true,
  enableClaude: true,
  enableSider: true,
  enableChatglm: true,
  enableCustom: false,
};

export const storage = {
  async get<T>(key: string): Promise<T | null> {
    const result = await chrome.storage.local.get(key);
    return (result[key] as T) ?? null;
  },

  async set<T>(key: string, value: T): Promise<void> {
    await chrome.storage.local.set({ [key]: value });
  },

  async remove(key: string): Promise<void> {
    await chrome.storage.local.remove(key);
  },

  async getConfig(): Promise<AgtalkConfig> {
    const saved = await this.get<AgtalkConfig>(StorageKeys.CONFIG);
    return { ...DEFAULT_CONFIG, ...(saved || {}) };
  },

  async saveConfig(patch: Partial<AgtalkConfig>): Promise<AgtalkConfig> {
    const next = { ...(await this.getConfig()), ...patch };
    await this.set(StorageKeys.CONFIG, next);
    return next;
  },

  async getSession(): Promise<AgtalkSession | null> {
    const raw = await this.get<LegacySessionShape>(StorageKeys.SESSION);
    return normalizeSession(raw);
  },

  async saveSession(value: AgtalkSession): Promise<void> {
    await this.set(StorageKeys.SESSION, value);
  },
};
