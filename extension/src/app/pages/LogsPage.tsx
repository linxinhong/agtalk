import { RefreshCw, FileText } from 'lucide-react';
import { useEffect } from 'react';
import { useAppStore } from '../store';

export function LogsPage() {
  const store = useAppStore();

  useEffect(() => {
    store.loadLogs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="p-4 space-y-3 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold text-gray-900 flex items-center gap-2">
          <FileText size={18} />
          日志
        </h2>
        <button
          onClick={() => store.loadLogs()}
          disabled={store.loading}
          className="inline-flex items-center gap-1.5 rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
        >
          <RefreshCw size={14} className={store.loading ? 'animate-spin' : ''} />
          刷新
        </button>
      </div>

      {store.logsError ? (
        <div className="bg-orange-50 border border-orange-200 rounded-lg p-4 text-sm text-orange-700">
          日志接口暂未暴露：{store.logsError}
        </div>
      ) : store.logs.length === 0 ? (
        <div className="bg-white border border-gray-200 rounded-lg p-6 text-center text-sm text-gray-400">
          暂无日志
        </div>
      ) : (
        <div className="bg-white border border-gray-200 rounded-lg overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-gray-50 border-b border-gray-200">
              <tr>
                <th className="text-left px-3 py-2 text-xs font-medium text-gray-500">时间</th>
                <th className="text-left px-3 py-2 text-xs font-medium text-gray-500">级别</th>
                <th className="text-left px-3 py-2 text-xs font-medium text-gray-500">消息</th>
              </tr>
            </thead>
            <tbody>
              {store.logs.map((log, idx) => (
                <tr key={idx} className="border-b border-gray-100 last:border-0">
                  <td className="px-3 py-2 text-gray-600 whitespace-nowrap">{log.timestamp}</td>
                  <td className="px-3 py-2">
                    <span className={`text-[10px] px-1.5 py-0.5 rounded ${levelClass(log.level)}`}>{log.level}</span>
                  </td>
                  <td className="px-3 py-2 text-gray-700">{log.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function levelClass(level: string): string {
  switch (level?.toLowerCase()) {
    case 'error':
      return 'bg-red-100 text-red-700';
    case 'warn':
      return 'bg-yellow-100 text-yellow-700';
    case 'info':
      return 'bg-blue-100 text-blue-700';
    default:
      return 'bg-gray-100 text-gray-600';
  }
}
