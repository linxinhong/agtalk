import type { AgtalkConfig } from '../storage/storage';

export type ApiResult<T> =
  | { ok: true; data: T }
  | { ok: false; error: { code: string; message: string; status?: number } };

export interface ApiErrorShape {
  code: string;
  message: string;
  status?: number;
}

// Daemon ClientMsg 语义（POST /api，单一端点）
export interface ClientMsgBase {
  type: string;
}

export interface PingClientMsg extends ClientMsgBase {
  type: 'ping';
}

export interface ListConversationsClientMsg extends ClientMsgBase {
  type: 'list_conversations';
  participant?: string | null;
}

export interface GetMessagesClientMsg extends ClientMsgBase {
  type: 'get_messages';
  conversation_id: string;
  participant?: string | null;
  limit?: number;
  before?: string | null;
}

export interface GetMessageClientMsg extends ClientMsgBase {
  type: 'get_message';
  msg_id: string;
  participant?: string | null;
}

export interface SendClientMsg extends ClientMsgBase {
  type: 'send';
  sender?: string | null;
  to: string;
  body: string;
  conversation_id?: string | null;
  reply_to?: string | null;
  correlation_id?: string | null;
  content_type?: string;
  metadata?: Record<string, unknown> | null;
  notify?: boolean;
}

export type ClientMsg =
  | PingClientMsg
  | ListConversationsClientMsg
  | GetMessagesClientMsg
  | GetMessageClientMsg
  | SendClientMsg;

export interface ServerOk<T = unknown> {
  type: 'ok';
  data: T;
}

export interface ServerError {
  type: 'error';
  code: string;
  message: string;
}

export type ServerMsg<T = unknown> = ServerOk<T> | ServerError;

export interface HealthResponse {
  pong: boolean;
}

export interface StatusResponse {
  connected: boolean;
  url: string;
  agentName?: string;
  sessionPresent: boolean;
  configPresent: boolean;
  inboxUnread?: number;
  inboxTotal?: number;
  peersOnline?: number;
  authError?: string;
  inboxError?: string;
  peersError?: string;
  error?: string;
}

export type ConfigResponse = AgtalkConfig;

export interface MessagePreview {
  id: string;
  sender_name: string;
  body: string;
  created_at: string;
}

export interface MessageCounts {
  unread: number;
  pending: number;
}

export interface Conversation {
  id: string;
  title: string;
  kind: string;
  peers: string[];
  last_message?: MessagePreview | null;
  counts: MessageCounts;
  created_at: string;
  updated_at: string;
}

export interface RecipientStatus {
  recipient_id: string;
  recipient_name: string;
  status: string;
  delivered_at?: string | null;
  read_at?: string | null;
  done_at?: string | null;
}

export interface Message {
  id: string;
  chat_id: string;
  subject?: string | null;
  sender_id: string;
  sender_name: string;
  body: string;
  body_size: number;
  content_type: string;
  status: string;
  correlation_id?: string | null;
  reply_to_id?: string | null;
  metadata?: Record<string, unknown> | null;
  recipients: RecipientStatus[];
  created_at: string;
}

export interface SendReplyResponse {
  message?: { id?: string };
  id?: string;
}

export interface LogItem {
  // daemon 目前没有日志端点，保留接口占位
  timestamp: string;
  level: string;
  message: string;
}
