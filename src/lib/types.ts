// 与 daemon 数据模型对应的 TypeScript 类型

export interface Participant {
  id: string;
  name: string;
  type: "agent" | "human";
  display_name: string;
  capabilities: string;
  transport: string;
  transport_config: string;
  status: "online" | "offline";
  last_seen_at: number;
  created_at: number;
}

export interface Conversation {
  id: string;
  title: string;
  kind: "direct" | "group" | "task" | "approval" | "incident";
  participants: string[];
  last_message: MessagePreview | null;
  unread_count: number;
  created_at: number;
  updated_at: number;
}

export interface MessagePreview {
  id: string;
  sender_name: string;
  body: string;
  created_at: number;
}

export interface RecipientStatus {
  recipient_id: string;
  recipient_name: string;
  status: "pending" | "delivered" | "read" | "done";
  delivered_at: number | null;
  read_at: number | null;
}

export interface Message {
  id: string;
  conversation_id: string;
  sender_id: string;
  sender_name: string;
  body: string;
  content_type: string;
  metadata: string;
  status: string;
  correlation_id: string | null;
  reply_to_id: string | null;
  recipients: RecipientStatus[];
  created_at: number;
}

export interface ServerResponse {
  type: "ok" | "error" | "event";
  data?: any;
  code?: string;
  message?: string;
  event?: string;
}
