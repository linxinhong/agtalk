import { useEffect } from 'react';
import { RefreshCw, Users, Settings, Server, Layers, Activity, ExternalLink, Zap, Unplug } from 'lucide-react';
import { Header } from '../components/Header';
import { StatusBar } from '../components/StatusBar';
import { ErrorBox } from '../components/ErrorBox';
import { Toggle } from '../components/Toggle';
import { usePopupStore } from '../store';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';

export function HomePage() {
  const store = usePopupStore();
  const config = store.config;
  const status = store.status;
  const connected = status?.ok && status.data.connected;

  useEffect(() => {
    store.loadAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openApp = async () => {
    await sendMessage({ type: MessageType.OPEN_APP_PAGE });
  };

  const MenuItem = ({
    icon: Icon,
    label,
    page,
  }: {
    icon: React.ElementType;
    label: string;
    page: Parameters<typeof store.navigate>[0];
  }) => (
    <button
      onClick={() => store.navigate(page)}
      className="flex items-center gap-2 px-3 py-2 rounded-md hover:bg-gray-100 text-sm text-gray-700"
    >
      <Icon size={16} />
      {label}
    </button>
  );

  return (
    <div className="flex flex-col h-full bg-gray-50">
      <Header title="agtalk" />
      <StatusBar status={status} />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="bg-white rounded-lg border border-gray-200 p-3">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs font-semibold text-gray-500 uppercase tracking-wide">自动注入</span>
            <button
              onClick={() => store.loadAll()}
              disabled={store.loading}
              className="p-1 rounded hover:bg-gray-100 text-gray-500 disabled:opacity-50"
              title="刷新"
            >
              <RefreshCw size={14} className={store.loading ? 'animate-spin' : ''} />
            </button>
          </div>
          <Toggle
            label="自动注入新消息到对话"
            checked={!!config?.autoInject}
            onChange={(v) => store.setAutoInject(v)}
            disabled={store.loading}
          />
          <p className="text-[11px] text-gray-400 mt-1">关闭时会清空 peer 级自动注入列表</p>
        </div>

        <div className="bg-white rounded-lg border border-gray-200 p-2 grid grid-cols-2 gap-1">
          <MenuItem icon={Users} label="Agent 管理" page="agents" />
          <MenuItem icon={Settings} label="Agent 配置" page="agentConfig" />
          <MenuItem icon={Server} label="本地服务" page="localService" />
          <MenuItem icon={Layers} label="平台开关" page="platformConfig" />
          <MenuItem icon={Activity} label="调试" page="debug" />
          <button
            onClick={openApp}
            className="flex items-center gap-2 px-3 py-2 rounded-md hover:bg-gray-100 text-sm text-gray-700"
          >
            <ExternalLink size={16} />
            打开 App
          </button>
        </div>

        <div className="flex gap-2">
          <button
            onClick={() => store.registerAgent()}
            disabled={store.loading || !config?.agentName}
            className="flex-1 inline-flex items-center justify-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 disabled:bg-gray-400"
          >
            <Zap size={14} />
            Join
          </button>
          <button
            onClick={() => store.reconnect()}
            disabled={store.loading || !config?.agentName}
            className="flex-1 inline-flex items-center justify-center gap-1.5 rounded-md border border-gray-300 px-3 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
          >
            <Unplug size={14} />
            重连
          </button>
        </div>

        {!config?.agentName && (
          <p className="text-xs text-orange-600 bg-orange-50 border border-orange-100 rounded-md p-2">
            请在「Agent 配置」中设置 agentName 后再 Join。
          </p>
        )}

        {connected && status?.ok && (
          <div className="text-xs text-gray-500 bg-white rounded-lg border border-gray-200 p-3 space-y-1">
            <p>daemon: {status.data.url}</p>
            {status.data.authError && <p className="text-orange-600">auth: {status.data.authError}</p>}
            {status.data.inboxError && <p className="text-orange-600">inbox: {status.data.inboxError}</p>}
            {status.data.peersError && <p className="text-orange-600">peers: {status.data.peersError}</p>}
          </div>
        )}
      </div>
    </div>
  );
}
