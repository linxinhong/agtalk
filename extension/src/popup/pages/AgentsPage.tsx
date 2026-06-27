import { useEffect } from 'react';
import { RefreshCw, Users, Check } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { PeerRow } from '../components/PeerRow';
import { usePopupStore } from '../store';

export function AgentsPage() {
  const store = usePopupStore();
  const peers = store.peers;
  const config = store.config;
  const globalAutoInject = !!config?.autoInject;
  const agentName = config?.agentName || '';

  useEffect(() => {
    store.loadPeers();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const allPeers = (peers?.peers || []).filter((p) => p.name !== agentName);
  const connected = allPeers.filter((p) => p.connected);
  const available = allPeers.filter((p) => !p.connected);

  return (
    <div className="flex flex-col h-full bg-gray-100">
      <Header
        title="Agent 管理"
        showBack
        onBack={() => store.back()}
        rightActions={
          <button
            onClick={() => store.loadPeers()}
            disabled={store.loading}
            className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500 disabled:opacity-50"
            title="刷新"
          >
            <RefreshCw size={16} className={store.loading ? 'animate-spin' : ''} />
          </button>
        }
      />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="flex items-center justify-between text-xs text-gray-500 px-1">
          <span>当前 Agent</span>
          <span className="font-medium text-gray-700">{agentName || '未配置'}</span>
        </div>

        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-gray-600 uppercase tracking-wide">
            <Users size={14} />
            已连接 ({connected.length})
          </div>
          <div className="space-y-2">
            {connected.length === 0 ? (
              <div className="text-xs text-gray-400 py-3 text-center border border-dashed border-gray-200 rounded-md">
                尚未连接 Agent
              </div>
            ) : (
              connected.map((p) => (
                <PeerRow
                  key={p.id}
                  peer={p}
                  globalAutoInject={globalAutoInject}
                  isActive={config?.activePeer === p.name}
                  onActivate={() => store.setActivePeer(p.name)}
                  onDisconnect={() => store.disconnectPeer(p.name)}
                  onToggleAutoInject={() => store.togglePeerAutoInject(p.name)}
                />
              ))
            )}
          </div>
        </section>

        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-gray-600 uppercase tracking-wide">
            <Users size={14} />
            可连接 ({available.length})
          </div>
          <div className="space-y-2">
            {available.length === 0 ? (
              <div className="text-xs text-gray-400 py-3 text-center border border-dashed border-gray-200 rounded-md">
                没有可连接的 Agent
              </div>
            ) : (
              available.map((p) => (
                <PeerRow
                  key={p.id}
                  peer={p}
                  globalAutoInject={globalAutoInject}
                  isActive={false}
                  onActivate={() => {}}
                  onConnect={() => store.connectPeer(p.name)}
                  onToggleAutoInject={() => {}}
                />
              ))
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
