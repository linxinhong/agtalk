export interface Mailbox {
  address: string;
  name: string;
  intro: string;
  created_at: number;
  left_at?: number;
  notify_channel: string;
  notify: string;
  notify_ready: boolean;
}

export interface Message {
  id: string;
  to_address: string;
  to_name: string;
  from_address: string;
  from_name: string;
  body: string;
  content_type: string;
  reply_to_id?: string;
  metadata: string;
  event_id: number;
  status: string;
  created_at: number;
}

export interface JoinResult {
  address: string;
  name: string;
  token: string;
}

export interface StoredSession {
  address: string;
  token: string;
  name?: string;
}
