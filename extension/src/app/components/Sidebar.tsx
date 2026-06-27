import { MessageSquare, Settings, FileText, Activity, RefreshCw } from 'lucide-react';
import { useAppStore, type AppPage } from '../store';

const navItems: { page: AppPage; label: string; icon: React.ElementType }[] = [
  { page: 'conversations', label: '会话 Conversations', icon: MessageSquare },
  { page: 'settings', label: '设置 Settings', icon: Settings },
  { page: 'logs', label: '日志 Logs', icon: FileText },
  { page: 'status', label: '状态 Status', icon: Activity },
];

export function Sidebar() {
  const store = useAppStore();
  const activePage = store.activePage;
  const status = store.status;
  const config = store.config;

  const connected = status?.ok && status.data.connected;
  const statusDotClass = connected ? 'bg-green-600' : status?.ok ? 'bg-yellow-500' : 'bg-red-500';
  const statusLabel = connected ? '在线' : status?.ok ? '异常' : '离线';

  return (
    <aside className="w-60 flex-shrink-0 bg-white border-r border-gray-200 flex flex-col h-screen">
      <div className="p-4 border-b border-gray-200">
        <h1 className="text-lg font-bold text-gray-900">agtalk</h1>
        <p className="text-xs text-gray-500">Web Bridge Console</p>
        <div className="mt-3 flex items-center gap-2 text-xs">
          <span className={`w-2 h-2 rounded-full ${statusDotClass}`} />
          <span className="text-gray-600">{statusLabel}</span>
        </div>
      </div>

      <nav className="flex-1 p-2 space-y-1">
        {navItems.map((item) => {
          const Icon = item.icon;
          const active = activePage === item.page;
          return (
            <button
              key={item.page}
              onClick={() => store.setActivePage(item.page)}
              className={`w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium transition-colors ${
                active
                  ? 'bg-blue-50 text-blue-700'
                  : 'text-gray-700 hover:bg-gray-100'
              }`}
            >
              <Icon size={16} />
              {item.label}
            </button>
          );
        })}
      </nav>

      <div className="p-3 border-t border-gray-200 space-y-2">
        <div className="text-xs text-gray-500">
          <p className="font-medium text-gray-700 truncate">{config?.agentName || '未配置 Agent'}</p>
          <p>inbox {status?.ok ? `${status.data.inboxUnread ?? 0}/${status.data.inboxTotal ?? 0}` : '-'}</p>
        </div>
        <button
          onClick={() => store.bootstrap()}
          disabled={store.loading}
          className="w-full flex items-center justify-center gap-1.5 rounded-md border border-gray-300 px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
        >
          <RefreshCw size={12} className={store.loading ? 'animate-spin' : ''} />
          刷新
        </button>
      </div>
    </aside>
  );
}
