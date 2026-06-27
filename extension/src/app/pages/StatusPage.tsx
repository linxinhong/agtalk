import { Activity, HeartPulse, Gauge, RefreshCw, Zap, Server, User, Inbox, Users } from 'lucide-react';
import { useEffect } from 'react';
import { useAppStore } from '../store';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';
import { useState } from 'react';

export function StatusPage() {
  const store = useAppStore();
  const [pong, setPong] = useState<boolean | null>(null);

  useEffect(() => {
    store.bootstrap();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ping = async () => {
    const res = await sendMessage<unknown, { pong?: boolean }>({ type: MessageType.PING_BACKGROUND });
    setPong(res?.pong ?? false);
  };

  const status = store.status;
  const config = store.config;

  return (
    <div className="p-4 space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold text-gray-900">状态</h2>
        <div className="flex gap-2">
          <ActionButton icon={HeartPulse} label="Health" onClick={() => store.loadHealth()} />
          <ActionButton icon={Gauge} label="Status" onClick={() => store.loadStatus()} />
          <ActionButton icon={Activity} label="Ping" onClick={ping} />
          <ActionButton icon={RefreshCw} label="Reload" onClick={() => store.bootstrap()} />
        </div>
      </div>

      {pong !== null && (
        <div className="bg-white rounded-lg border border-gray-200 p-3 text-sm">
          Ping background: <span className={pong ? 'text-green-600' : 'text-red-500'}>{pong ? 'pong' : 'no response'}</span>
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <StatusCard icon={Server} title="Daemon" ok={status?.ok}>
          {status?.ok ? (
            <>
              <p>URL: {status.data.url}</p>
              <p>状态: 在线</p>
            </>
          ) : (
            <p>{status?.error.code || '未连接'}: {status?.error.message}</p>
          )}
        </StatusCard>

        <StatusCard icon={User} title="Agent" ok={!!config?.agentName}>
          <p>名称: {config?.agentName || '未配置'}</p>
          <p>角色: {config?.agentRole || '-'}</p>
          <p>启用: {config?.enabled ? '是' : '否'}</p>
        </StatusCard>

        <StatusCard icon={Inbox} title="Inbox" ok={status?.ok}>
          {status?.ok ? (
            <>
              <p>未读/总计: {status.data.inboxUnread ?? 0}/{status.data.inboxTotal ?? 0}</p>
              {status.data.authError && <p className="text-orange-600">auth: {status.data.authError}</p>}
              {status.data.inboxError && <p className="text-orange-600">inbox: {status.data.inboxError}</p>}
            </>
          ) : (
            <p>-</p>
          )}
        </StatusCard>

        <StatusCard icon={Users} title="Peers" ok={status?.ok}>
          {status?.ok ? (
            <>
              <p>在线: {status.data.peersOnline ?? 0}</p>
              {status.data.peersError && <p className="text-orange-600">peers: {status.data.peersError}</p>}
            </>
          ) : (
            <p>-</p>
          )}
        </StatusCard>
      </div>

      {config && (
        <div className="bg-white rounded-lg border border-gray-200 p-4 text-sm space-y-1 text-gray-600">
          <p><span className="font-medium">sessionPresent:</span> {status?.ok && status.data.sessionPresent ? '是' : '否'}</p>
          <p><span className="font-medium">configPresent:</span> {status?.ok && status.data.configPresent ? '是' : '否'}</p>
          <p><span className="font-medium">workspace:</span> {config.workspaceName} ({config.workspaceRoot})</p>
          <p><span className="font-medium">pollInterval:</span> {config.pollInterval}ms</p>
        </div>
      )}
    </div>
  );
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
}: {
  icon: React.ElementType;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="inline-flex items-center gap-1.5 rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50"
    >
      <Icon size={14} />
      {label}
    </button>
  );
}

function StatusCard({
  icon: Icon,
  title,
  ok,
  children,
}: {
  icon: React.ElementType;
  title: string;
  ok?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="bg-white rounded-lg border border-gray-200 p-4">
      <div className="flex items-center gap-2 mb-2">
        <Icon size={16} className="text-gray-500" />
        <h3 className="text-sm font-semibold text-gray-900">{title}</h3>
        <span className={`ml-auto w-2 h-2 rounded-full ${ok ? 'bg-green-600' : 'bg-red-500'}`} />
      </div>
      <div className="text-sm text-gray-600 space-y-0.5">{children}</div>
    </div>
  );
}
