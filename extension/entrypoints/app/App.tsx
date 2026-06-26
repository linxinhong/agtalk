import { useState } from 'react';
import { Activity, HeartPulse, Gauge } from 'lucide-react';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';
import type { ApiResult, HealthResponse, StatusResponse } from '@/shared/api/types';

function App() {
  const [pong, setPong] = useState<boolean | null>(null);
  const [health, setHealth] = useState<ApiResult<HealthResponse> | null>(null);
  const [status, setStatus] = useState<ApiResult<StatusResponse> | null>(null);

  const pingBackground = async () => {
    const res = await sendMessage<unknown, { pong?: boolean }>({ type: MessageType.PING_BACKGROUND });
    setPong(res?.pong ?? false);
  };

  const checkHealth = async () => {
    const res = await sendMessage<unknown, ApiResult<HealthResponse>>({
      type: MessageType.API_HEALTH_CHECK,
    });
    setHealth(res ?? null);
  };

  const checkStatus = async () => {
    const res = await sendMessage<unknown, ApiResult<StatusResponse>>({
      type: MessageType.API_GET_STATUS,
    });
    setStatus(res ?? null);
  };

  return (
    <div className="min-h-screen p-8 space-y-4">
      <h1 className="text-2xl font-bold text-gray-900">agtalk App</h1>
      <p className="text-gray-600">Phase 2 shared layer</p>

      <div className="flex gap-2">
        <button
          onClick={pingBackground}
          className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
        >
          <Activity size={16} />
          Ping Background
        </button>
        <button
          onClick={checkHealth}
          className="inline-flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-700"
        >
          <HeartPulse size={16} />
          Health
        </button>
        <button
          onClick={checkStatus}
          className="inline-flex items-center gap-2 rounded-md bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-700"
        >
          <Gauge size={16} />
          Status
        </button>
      </div>

      {pong !== null && (
        <p className="text-sm text-gray-500">Background: {pong ? 'pong' : 'no response'}</p>
      )}

      {health && (
        <div className="text-sm rounded border border-gray-200 p-3 bg-gray-50 max-w-md">
          <p className="font-medium text-gray-700">Health:</p>
          {health.ok ? (
            <p className="text-emerald-700">daemon pong = {String(health.data.pong)}</p>
          ) : (
            <p className="text-red-600">{health.error.code}: {health.error.message}</p>
          )}
        </div>
      )}

      {status && (
        <div className="text-sm rounded border border-gray-200 p-3 bg-gray-50 max-w-md">
          <p className="font-medium text-gray-700">Status:</p>
          {status.ok ? (
            <div className="space-y-0.5 text-gray-600">
              <p>connected: {String(status.data.connected)}</p>
              <p>agent: {status.data.agentName || '-'}</p>
              <p>inbox: {status.data.inboxUnread ?? 0}/{status.data.inboxTotal ?? 0}</p>
              <p>peers online: {status.data.peersOnline ?? 0}</p>
              {status.data.authError && <p className="text-orange-600">auth: {status.data.authError}</p>}
              {status.data.inboxError && <p className="text-orange-600">inbox: {status.data.inboxError}</p>}
              {status.data.peersError && <p className="text-orange-600">peers: {status.data.peersError}</p>}
            </div>
          ) : (
            <p className="text-red-600">{status.error.code}: {status.error.message}</p>
          )}
        </div>
      )}
    </div>
  );
}

export default App;
