import type { MessageTypeValue } from './message-types';

export type BackgroundHandler<T = unknown, R = unknown> = (
  message: { type: MessageTypeValue; payload?: T },
  sender: chrome.runtime.MessageSender
) => Promise<R> | R;

export interface HandlerRegistry {
  [type: string]: BackgroundHandler<unknown, unknown> | undefined;
}

export function createBackgroundRouter(handlers: HandlerRegistry) {
  return (
    message: { type: MessageTypeValue; payload?: unknown },
    sender: chrome.runtime.MessageSender,
    sendResponse: (response: unknown) => void
  ): boolean => {
    const handler = handlers[message.type];
    if (!handler) return false;

    Promise.resolve(handler(message, sender))
      .then((result) => sendResponse(result))
      .catch((err) =>
        sendResponse({
          ok: false,
          error: err instanceof Error ? err.message : String(err),
        })
      );

    return true;
  };
}
