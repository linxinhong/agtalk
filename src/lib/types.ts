// 与 daemon 数据模型对应的 TypeScript 类型

export interface Participant {
  id: string;
  name: string;
  type: "agent" | "human";
  display_name: string;
  intro: string;
  capabilities: string;
  transport: string;
  transport_config: string;
  role?: string;
  status: "online" | "offline";
  last_seen_at: string;
  created_at: string;
}

export interface Conversation {
  id: string;
  title: string;
  kind: "direct" | "group" | "task" | "approval" | "incident";
  participants: string[];
  last_message: MessagePreview | null;
  unread_count: number;
  created_at: string;
  updated_at: string;
}

export interface MessagePreview {
  id: string;
  sender_name: string;
  body: string;
  created_at: string;
}

export interface Attachment {
  id: string;
  message_id: string;
  role:
    | "full_body"
    | "user_file"
    | "generated_report"
    | "patch"
    | "log"
    | "artifact"
    | "attachment";
  filename: string;
  content_type: string;
  size: number;
  created_at: string;
}

export interface RecipientStatus {
  recipient_id: string;
  recipient_name: string;
  status: "pending" | "delivered" | "read" | "done";
  delivered_at: string | null;
  read_at: string | null;
  done_at: string | null;
  read_by_session_id: string | null;
  done_by_session_id: string | null;
}

export interface Message {
  id: string;
  chat_id: string;
  sender_id: string;
  sender_name: string;
  subject?: string | null;
  body: string;
  body_size: number;
  content_type: string;
  metadata: string;
  status: string;
  correlation_id: string | null;
  reply_to_id: string | null;
  recipients: RecipientStatus[];
  attachments: Attachment[];
  full_body: string | null;
  created_at: string;
}

export interface InboxSender {
  id: string;
  name: string;
  type: "agent" | "human";
  intro: string;
}

export interface InboxMessageContent {
  mode: "full" | "preview" | "summary";
  body: string;
  truncated: boolean;
  size: number;
}

export interface InboxAttachment {
  id: string;
  role: Attachment["role"];
  filename: string;
  content_type: string;
  size: number;
}

export interface InboxDelivery {
  status: "pending" | "delivered" | "read" | "done";
  delivered_at: string | null;
  read_at: string | null;
  done_at: string | null;
  read_by_session_id: string | null;
  done_by_session_id: string | null;
}

export interface InboxItem {
  id: string;
  from: InboxSender;
  subject: string | null;
  content: InboxMessageContent;
  attachments: InboxAttachment[];
  delivery: InboxDelivery;
  actions: string[];
  action_required: boolean;
  priority: "high" | "normal";
  kind: "approval" | "question" | "task" | "message";
}

export interface ServerResponse {
  type: "ok" | "error" | "event";
  data?: any;
  code?: string;
  message?: string;
  event?: string;
}
