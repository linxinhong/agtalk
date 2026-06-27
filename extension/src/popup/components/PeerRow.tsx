import type { Peer } from '@/shared/api/types';
import { Toggle } from './Toggle';

interface PeerRowProps {
  peer: Peer & { connected?: boolean; autoInject?: boolean };
  globalAutoInject: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onToggleAutoInject: () => void;
}

export function PeerRow({ peer, globalAutoInject, onConnect, onDisconnect, onToggleAutoInject }: PeerRowProps) {
  const description = [peer.participant_type, peer.role, peer.status, peer.transport]
    .filter(Boolean)
    .join(' / ');

  return (
    <div className="flex items-center justify-between gap-3 py-2 px-3 border border-gray-200 rounded-md bg-white">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold text-gray-900 truncate">{peer.name}</div>
        <div className="text-xs text-gray-500 truncate">{description || 'peer'}</div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        {peer.connected ? (
          <>
            <Toggle
              label=""
              checked={peer.autoInject || false}
              onChange={onToggleAutoInject}
              disabled={!globalAutoInject}
            />
            <button
              onClick={onDisconnect}
              className="px-2 py-1 text-xs rounded border border-gray-300 text-gray-700 hover:bg-gray-50"
            >
              断开
            </button>
          </>
        ) : (
          <button
            onClick={onConnect}
            className="px-2 py-1 text-xs rounded bg-blue-600 text-white hover:bg-blue-700"
          >
            连接
          </button>
        )}
      </div>
    </div>
  );
}
