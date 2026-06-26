export const BASE_URL = 'http://127.0.0.1:19527';

export const DEFAULT_TIMEOUT_MS = 10000;

export interface ApiRequestOptions {
  type: string;
  payload?: Record<string, unknown>;
  needsAuth?: boolean;
  timeoutMs?: number;
}

export function getApiUrl(): string {
  return `${BASE_URL}/api`;
}
