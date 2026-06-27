import { useEffect } from 'react';
import { RefreshCw, UserPlus } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { PeerRow } from '../components/PeerRow';
import { usePopupStore } from '../store';

export function AgentsPage() {
  const store = usePopupStore();
  const peers = store.peers;
  const globalAutoInject = !!store.config?.autoInject;

  useEffect(() => {
    store.loadPeers();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connected = (peers?.peers || []).filter((p) => p.connected);
  const available = (peers?.peers || []).filter((p) => !p.connected);

  return (
    <div className="flex flex-col h-full bg-gray-50">
      <Header title="Agent 管理" showBack onBack={() => store.back()} />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="px-3 py-2 bg-white border-b border-gray-200 flex items-center justify-between">
        <span className="text-xs text-gray-500">
          {peers?.agentName ? `当前: ${peers.agentName}` : '未注册 Agent'}
        </span>
        <button
          onClick={() => store.loadPeers()}
          disabled={store.loading}
          className="p-1 rounded hover:bg-gray-100 text-gray-500 disabled:opacity-50"
        >
          <RefreshCw size={14} className={store.loading ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <section>
          <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">已连接 ({connected.length})</h3>
          <div className="space-y-2">
            {connected.length === 0 ? (
              <div className="text-sm text-gray-400 py-4 text-center bg-white rounded-lg border border-gray-200 border-dashed">
                尚未连接 Agent
              </div>
            ) : (
              connected.map((p) => (
                <PeerRow
                  key={p.id}
                  peer={p}
                  globalAutoInject={globalAutoInject}
                  onConnect={() => {}}
                  onDisconnect={() => store.disconnectPeer(p.name)}
                  onToggleAutoInject={() => store.togglePeerAutoInject(p.name)}
                />
              ))
            )}
          </div>
        </section>

        <section>
          <h3 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">可连接 ({available.length})</h3>
          <div className="space-y-2">
            {available.length === 0 ? (
              <div className="text-sm text-gray-400 py-4 text-center bg-white rounded-lg border border-gray-200 border-dashed">
                没有可连接的 Agent
              </div>
            ) : (
              available.map((p) => (
                <PeerRow
                  key={p.id}
                  peer={p}
                  globalAutoInject={globalAutoInject}
                  onConnect={() => store.connectPeer(p.name)}
                  onDisconnect={() => {}}
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
