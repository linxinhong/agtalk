// Tauri IPC 封装：调用 Rust 后端 commands，与 daemon 通信

import { invoke } from "@tauri-apps/api/core";
import type { Conversation, Message, Participant, ServerResponse } from "./types";

function parseResponse<T>(json: string): T {
  const resp: ServerResponse = JSON.parse(json);
  if (resp.type === "error") {
    throw new Error(resp.message || "未知错误");
  }
  return (resp.data || resp) as T;
}

export async function listConversations(
  participant?: string
): Promise<Conversation[]> {
  const json = await invoke<string>("list_conversations", {
    participant: participant || null,
  });
  return parseResponse<Conversation[]>(json);
}

export async function getMessages(
  conversationId: string,
  limit?: number,
  before?: string
): Promise<Message[]> {
  const json = await invoke<string>("get_messages", {
    conversationId,
    limit: limit || 50,
    before: before || null,
  });
  return parseResponse<Message[]>(json);
}

export async function sendMessage(
  to: string,
  body: string,
  conversationId?: string,
  replyTo?: string,
  contentType?: string,
  sender?: string
): Promise<Message> {
  const json = await invoke<string>("send_message", {
    payload: {
      to, body,
      conversation_id: conversationId,
      reply_to: replyTo,
      content_type: contentType,
      sender,
    },
  });
  return parseResponse<Message>(json);
}

export async function markDone(
  msgId: string,
  participant: string
): Promise<void> {
  const json = await invoke<string>("mark_done", {
    msgId,
    participant,
  });
  parseResponse(json);
}

export async function markRead(
  msgId: string,
  participant: string
): Promise<void> {
  const json = await invoke<string>("mark_read", {
    msgId,
    participant,
  });
  parseResponse(json);
}

export async function listParticipants(
  type?: string
): Promise<Participant[]> {
  const json = await invoke<string>("list_participants", {
    participantType: type || null,
  });
  return parseResponse<Participant[]>(json);
}

export async function pingDaemon(): Promise<boolean> {
  try {
    const json = await invoke<string>("ping_daemon");
    parseResponse(json);
    return true;
  } catch {
    return false;
  }
}

export async function replyApproval(
  msgId: string,
  choice: string,
  reason?: string
): Promise<void> {
  const json = await invoke<string>("reply", {
    msgId,
    choice,
    reason: reason ?? null,
  });
  parseResponse(json);
}
