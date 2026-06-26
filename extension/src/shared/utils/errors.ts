export interface AgtalkError {
  code: string;
  message: string;
  status?: number;
}

export function isError(err: unknown): err is Error {
  return err instanceof Error;
}

export function createError(code: string, message: string, status?: number): AgtalkError {
  return { code, message, status };
}

export function errorToString(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return '未知错误';
}
