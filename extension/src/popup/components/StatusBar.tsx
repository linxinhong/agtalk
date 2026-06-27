import type { ApiResult, StatusResponse } from '@/shared/api/types';

interface StatusBarProps {
  status: ApiResult<StatusResponse> | null;
}

export function StatusBar({ status }: StatusBarProps) {
  const connected = status?.ok && status.data.connected;
  const error = status?.ok ? status.data.error : status?.error.message;

  return (
    <div className="px-3 py-2 bg-white border-b border-gray-200 text-xs">
      <div className="flex items-center gap-2 mb-1">
        <span
          className={`w-2 h-2 rounded-full ${connected ? 'bg-green-600' : error ? 'bg-red-500' : 'bg-yellow-500'}`}
          title={connected ? '在线' : error ? '错误' : '连接中'}
        />
        <span className="font-medium text-gray-700 truncate">
          {status?.ok ? status.data.agentName || '未命名 Agent' : status?.error.code || '未连接'}
        </span>
      </div>
      <div className="flex justify-between text-gray-500">
        <span>inbox {status?.ok ? `${status.data.inboxUnread ?? 0}/${status.data.inboxTotal ?? 0}` : '-'}</span>
        <span>peers {status?.ok ? status.data.peersOnline ?? 0 : '-'}</span>
      </div>
    </div>
  );
}
