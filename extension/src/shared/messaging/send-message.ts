import type { MessageTypeValue } from './message-types';

export interface TypedMessage<T = unknown> {
  type: MessageTypeValue;
  payload?: T;
}

export type MessageResponse<T = unknown> =
  | { ok: true; data?: T }
  | { ok: false; error: string }
  | T;

export async function sendMessage<T = unknown, R = unknown>(
  message: TypedMessage<T>
): Promise<R | undefined> {
  return new Promise((resolve, reject) => {
    try {
      chrome.runtime.sendMessage(message, (response) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
          return;
        }
        resolve(response as R);
      });
    } catch (err) {
      reject(err);
    }
  });
}

export async function sendMessageToTab<T = unknown, R = unknown>(
  tabId: number,
  message: TypedMessage<T>
): Promise<R | undefined> {
  return new Promise((resolve, reject) => {
    try {
      chrome.tabs.sendMessage(tabId, message, (response) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
          return;
        }
        resolve(response as R);
      });
    } catch (err) {
      reject(err);
    }
  });
}
