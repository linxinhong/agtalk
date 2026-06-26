export interface TimeoutOptions {
  ms?: number;
  signal?: AbortSignal;
}

export function createTimeoutSignal(ms: number): AbortSignal {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  // Node/Browser 兼容：超时后清理定时器
  if (typeof controller.signal.addEventListener === 'function') {
    controller.signal.addEventListener('abort', () => clearTimeout(timer));
  }
  return controller.signal;
}

export function combineSignals(...signals: (AbortSignal | undefined)[]): AbortSignal {
  const controller = new AbortController();
  const cleanup = () => controller.abort();

  for (const signal of signals) {
    if (!signal) continue;
    if (signal.aborted) {
      controller.abort();
      break;
    }
    signal.addEventListener('abort', cleanup, { once: true });
  }

  return controller.signal;
}

export async function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  const timeoutSignal = createTimeoutSignal(ms);
  return Promise.race([
    promise,
    new Promise<never>((_, reject) => {
      timeoutSignal.addEventListener('abort', () => reject(new Error('timeout')), { once: true });
    }),
  ]);
}
