import { storage } from '../storage/storage';
import { errorToString } from '../utils/errors';
import { createTimeoutSignal } from '../utils/timeout';
import type {
  ApiResult,
  ClientMsg,
  ConfigResponse,
  Conversation,
  HealthResponse,
  InboxItem,
  Message,
  Peer,
  SendReplyResponse,
  ServerMsg,
  StatusResponse,
  LogItem,
} from './types';
import type { AgtalkConfig } from '../storage/storage';
import { BASE_URL, DEFAULT_TIMEOUT_MS, getApiUrl, type ApiRequestOptions } from './endpoints';

function sanitizeForLog(message: string): string {
  // 保守过滤：避免意外把 token/session 打到日志
  return message
    .replace(/token[\s="]+[^\s&"]+/gi, 'token=<redacted>')
    .replace(/session_id[\s="]+[^\s&"]+/gi, 'session_id=<redacted>');
}

function ok<T>(data: T): ApiResult<T> {
  return { ok: true, data };
}

function fail<T>(code: string, message: string, status?: number): ApiResult<T> {
  return { ok: false, error: { code, message, status } };
}

async function loadAuthHeaders(): Promise<
  { sessionId: string; token: string; participant?: string } | null
> {
  const session = await storage.getSession();
  if (!session?.session_id || !session?.token) return null;
  return {
    sessionId: session.session_id,
    token: session.token,
    participant: session.participant,
  };
}

export async function apiRequest<T>(options: ApiRequestOptions): Promise<ApiResult<T>> {
  const { type, payload = {}, needsAuth = true, timeoutMs = DEFAULT_TIMEOUT_MS } = options;

  let auth: Awaited<ReturnType<typeof loadAuthHeaders>> = null;
  if (needsAuth) {
    auth = await loadAuthHeaders();
    if (!auth) {
      return fail('session_missing', '未找到本地 session，请先 join/attach');
    }
  }

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (auth) {
    headers['Authorization'] = `Bearer ${auth.token}`;
    headers['X-Agtalk-Session-Id'] = auth.sessionId;
    headers['X-Agtalk-Token'] = auth.token;
  }

  const body: ClientMsg = { type, ...payload } as ClientMsg;
  const signal = createTimeoutSignal(timeoutMs);

  try {
    const response = await fetch(getApiUrl(), {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
      signal,
    });

    if (!response.ok) {
      const text = await response.text().catch(() => '');
      return fail(
        'http_error',
        sanitizeForLog(text) || `HTTP ${response.status}`,
        response.status
      );
    }

    const result = (await response.json()) as ServerMsg<T>;
    if (result.type === 'error') {
      return fail(result.code, result.message);
    }
    return ok(result.data as T);
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      return fail('timeout', `请求 ${type} 超时（${timeoutMs}ms）`);
    }
    return fail('fetch_error', sanitizeForLog(errorToString(err)));
  }
}

export async function getHealth(): Promise<ApiResult<HealthResponse>> {
  const res = await apiRequest<unknown>({ type: 'ping', needsAuth: false });
  if (!res.ok) return res as ApiResult<HealthResponse>;
  return ok({ pong: true });
}

export async function getStatus(): Promise<ApiResult<StatusResponse>> {
  const health = await getHealth();
  if (!health.ok) {
    return fail(health.error.code, health.error.message);
  }

  const [config, session] = await Promise.all([storage.getConfig(), storage.getSession()]);
  const participantName = session?.participant || config.agentName || '';

  let inboxUnread = 0;
  let inboxTotal = 0;
  let peersOnline = 0;
  let authError: string | undefined;
  let inboxError: string | undefined;
  let peersError: string | undefined;

  if (!session) {
    authError = '未找到本地 session';
  } else if (!participantName) {
    authError = 'session 中未包含 participant，且 config.agentName 为空';
  }

  // list_participants 在 daemon 端免认证，在线时即可查询
  const peersRes = await apiRequest<unknown[]>({
    type: 'list_participants',
    payload: { participant_type: null, include_deleted: false, active_only: true },
    needsAuth: false,
  });
  if (peersRes.ok) {
    const list = Array.isArray(peersRes.data) ? peersRes.data : [];
    peersOnline = list.filter((p: any) => p?.status === 'online').length;
  } else {
    peersError = peersRes.error.message;
  }

  // inbox 需要有效 session + participantName
  if (session && participantName) {
    const inboxRes = await apiRequest<unknown[]>({
      type: 'inbox',
      payload: { participant: participantName, status: 'all', limit: 1000, peek: true },
    });
    if (inboxRes.ok) {
      const items = Array.isArray(inboxRes.data) ? inboxRes.data : [];
      inboxTotal = items.length;
      inboxUnread = items.filter((i: any) => {
        const delivery = i?.delivery || (i?.recipients?.[0] ? { status: i.recipients[0].status, read_at: i.recipients[0].read_at } : {});
        return !delivery.read_at && (delivery.status === 'pending' || delivery.status === 'unread');
      }).length;
    } else {
      inboxError = inboxRes.error.message;
    }
  }

  return ok({
    connected: true,
    url: config.daemonUrl || BASE_URL,
    agentName: participantName,
    sessionPresent: !!session,
    configPresent: !!config,
    inboxUnread,
    inboxTotal,
    peersOnline,
    authError,
    inboxError,
    peersError,
  });
}

export async function getConfig(): Promise<ApiResult<ConfigResponse>> {
  try {
    const config = await storage.getConfig();
    return ok(config);
  } catch (err) {
    return fail('storage_error', errorToString(err));
  }
}

export async function saveConfig(patch: Partial<ConfigResponse>): Promise<ApiResult<ConfigResponse>> {
  try {
    const config = await storage.saveConfig(patch);
    return ok(config);
  } catch (err) {
    return fail('storage_error', errorToString(err));
  }
}

export async function getConversations(): Promise<ApiResult<Conversation[]>> {
  const session = await storage.getSession();
  const participant = session?.participant || (await storage.getConfig()).agentName || '';
  return apiRequest<Conversation[]>({
    type: 'list_conversations',
    payload: { participant: participant || null },
  });
}

export async function getConversationMessages(id: string): Promise<ApiResult<Message[]>> {
  const session = await storage.getSession();
  const participant = session?.participant || (await storage.getConfig()).agentName || '';
  return apiRequest<Message[]>({
    type: 'get_messages',
    payload: { conversation_id: id, participant: participant || null, limit: 50, before: null },
  });
}

export async function sendReply(replyToMsgId: string, text: string): Promise<ApiResult<SendReplyResponse>> {
  const session = await storage.getSession();
  const participant = session?.participant || (await storage.getConfig()).agentName || '';

  const detailRes = await apiRequest<{ sender_name?: string; from?: { name?: string } }>({
    type: 'get_message',
    payload: { msg_id: replyToMsgId, participant: participant || null },
  });
  if (!detailRes.ok) {
    return fail(detailRes.error.code, detailRes.error.message);
  }

  const to = detailRes.data.sender_name || detailRes.data.from?.name;
  if (!to) {
    return fail('recipient_unknown', '无法从原消息解析目标参与者');
  }

  return apiRequest<SendReplyResponse>({
    type: 'send',
    payload: {
      to,
      body: text,
      reply_to: replyToMsgId,
      content_type: 'text',
      notify: true,
      conversation_id: null,
      correlation_id: null,
      metadata: null,
    },
  });
}

export async function getLogs(): Promise<ApiResult<LogItem[]>> {
  // daemon 目前没有日志端点，返回结构化 not_implemented，不伪造成功
  return fail('not_implemented', 'daemon 暂未暴露日志查询接口');
}

export async function getInbox(limit = 5, statusFilter = 'all'): Promise<ApiResult<InboxItem[]>> {
  const session = await storage.getSession();
  const participant = session?.participant || (await storage.getConfig()).agentName || '';
  const res = await apiRequest<InboxItem[] | { items?: InboxItem[]; messages?: InboxItem[] }>({
    type: 'inbox',
    payload: { participant: participant || null, status: statusFilter, limit, peek: false },
  });
  if (!res.ok) return res as ApiResult<InboxItem[]>;
  const list = Array.isArray(res.data)
    ? res.data
    : res.data.items ?? res.data.messages ?? [];
  return ok(list);
}

export async function listParticipants(): Promise<ApiResult<Peer[]>> {
  const res = await apiRequest<Peer[] | { participants?: Peer[] }>({
    type: 'list_participants',
    payload: { participant_type: null, include_deleted: false, active_only: true },
    needsAuth: false,
  });
  if (!res.ok) return res as ApiResult<Peer[]>;
  const list = Array.isArray(res.data) ? res.data : res.data.participants ?? [];
  return ok(list);
}

export interface JoinResult {
  workspace_id: string;
  participant_id: string;
  session_id: string;
  token: string;
}

export async function join(name: string, config?: Partial<AgtalkConfig>): Promise<ApiResult<JoinResult>> {
  const cfg = config || (await storage.getConfig());
  const res = await apiRequest<JoinResult>({
    type: 'join',
    payload: {
      workspace_root: cfg.workspaceRoot || '/virtual/web-bridge',
      workspace_name: cfg.workspaceName || 'web-bridge',
      name,
      participant_type: 'web',
      role: cfg.agentRole || 'web',
      intro: cfg.agentBio || 'Web AI bridge participant',
      capabilities: cfg.agentCapabilities || '',
      transport: 'http',
      takeover: true,
      notify: false,
      timeout_ms: null,
    },
    needsAuth: false,
  });
  if (!res.ok) return res;

  await storage.saveSession({
    session_id: res.data.session_id,
    token: res.data.token,
    participant: name,
    workspace_id: res.data.workspace_id,
  });
  return res;
}
