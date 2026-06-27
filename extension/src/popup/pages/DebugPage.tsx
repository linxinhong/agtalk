import { useState } from 'react';
import { Activity, HeartPulse, Gauge, RefreshCw, ChevronRight } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { usePopupStore } from '../store';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';

export function DebugPage() {
  const store = usePopupStore();
  const [pong, setPong] = useState<boolean | null>(null);

  const pingBackground = async () => {
    const res = await sendMessage<unknown, { pong?: boolean }>({ type: MessageType.PING_BACKGROUND });
    setPong(res?.pong ?? false);
  };

  return (
    <div className="flex flex-col h-full bg-gray-100">
      <Header
        title="调试"
        showBack
        onBack={() => store.back()}
        rightActions={
          <button
            onClick={() => store.loadAll()}
            disabled={store.loading}
            className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500 disabled:opacity-50"
            title="重载"
          >
            <RefreshCw size={16} className={store.loading ? 'animate-spin' : ''} />
          </button>
        }
      />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="grid grid-cols-3 gap-2">
          <ActionButton icon={Activity} label="Ping" onClick={pingBackground} disabled={store.loading} />
          <ActionButton icon={HeartPulse} label="Health" onClick={() => store.loadHealth()} disabled={store.loading} />
          <ActionButton icon={Gauge} label="Status" onClick={() => store.loadStatus()} disabled={store.loading} />
        </div>

        {pong !== null && (
          <div className="bg-white rounded-lg border border-gray-200 p-2 text-sm">
            Ping background: <span className={pong ? 'text-green-600' : 'text-red-500'}>{pong ? 'pong' : 'no response'}</span>
          </div>
        )}

        {store.health && (
          <DebugCard title="Health" ok={store.health.ok}>
            {store.health.ok ? (
              <p>pong = {String(store.health.data.pong)}</p>
            ) : (
              <p>{store.health.error.code}: {store.health.error.message}</p>
            )}
          </DebugCard>
        )}

        {store.status && (
          <DebugCard title="Status" ok={store.status.ok}>
            {store.status.ok ? (
              <div className="space-y-0.5">
                <p>connected: {String(store.status.data.connected)}</p>
                <p>agent: {store.status.data.agentName || '-'}</p>
                <p>inbox: {store.status.data.inboxUnread ?? 0}/{store.status.data.inboxTotal ?? 0}</p>
                <p>peers online: {store.status.data.peersOnline ?? 0}</p>
                {store.status.data.authError && <p className="text-orange-600">auth: {store.status.data.authError}</p>}
                {store.status.data.inboxError && <p className="text-orange-600">inbox: {store.status.data.inboxError}</p>}
                {store.status.data.peersError && <p className="text-orange-600">peers: {store.status.data.peersError}</p>}
              </div>
            ) : (
              <p>{store.status.error.code}: {store.status.error.message}</p>
            )}
          </DebugCard>
        )}
      </div>
    </div>
  );
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
  disabled,
}: {
  icon: React.ElementType;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex flex-col items-center justify-center gap-1 rounded-lg border border-gray-200 bg-white py-2.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 disabled:bg-gray-100"
    >
      <Icon size={16} />
      {label}
    </button>
  );
}

function DebugCard({
  title,
  ok,
  children,
}: {
  title: string;
  ok: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="bg-white rounded-lg border border-gray-200 p-3 text-xs">
      <div className="flex items-center gap-2 mb-1.5">
        <ChevronRight size={14} className="text-gray-400" />
        <span className="font-semibold text-gray-700">{title}</span>
        <span className={`ml-auto w-2 h-2 rounded-full ${ok ? 'bg-green-600' : 'bg-red-500'}`} />
      </div>
      <div className="text-gray-600">{children}</div>
    </div>
  );
}
