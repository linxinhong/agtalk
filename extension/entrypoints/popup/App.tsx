import { useState } from 'react';
import { Activity, ExternalLink, HeartPulse, Gauge } from 'lucide-react';
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

  const openApp = async () => {
    await sendMessage({ type: MessageType.OPEN_APP_PAGE });
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
    <div className="w-80 p-4 space-y-3">
      <h1 className="text-lg font-semibold text-gray-900">agtalk Web Bridge</h1>
      <p className="text-sm text-gray-600">Phase 2 shared layer</p>

      <div className="grid grid-cols-2 gap-2">
        <button
          onClick={pingBackground}
          className="flex items-center justify-center gap-1.5 rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700"
        >
          <Activity size={14} />
          Ping BG
        </button>
        <button
          onClick={openApp}
          className="flex items-center justify-center gap-1.5 rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50"
        >
          <ExternalLink size={14} />
          Open App
        </button>
        <button
          onClick={checkHealth}
          className="flex items-center justify-center gap-1.5 rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-emerald-700"
        >
          <HeartPulse size={14} />
          Health
        </button>
        <button
          onClick={checkStatus}
          className="flex items-center justify-center gap-1.5 rounded-md bg-violet-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-violet-700"
        >
          <Gauge size={14} />
          Status
        </button>
      </div>

      {pong !== null && (
        <p className="text-xs text-gray-500">Background: {pong ? 'pong' : 'no response'}</p>
      )}

      {health && (
        <div className="text-xs rounded border border-gray-200 p-2 bg-gray-50">
          <p className="font-medium text-gray-700">Health:</p>
          {health.ok ? (
            <p className="text-emerald-700">daemon pong = {String(health.data.pong)}</p>
          ) : (
            <p className="text-red-600">{health.error.code}: {health.error.message}</p>
          )}
        </div>
      )}

      {status && (
        <div className="text-xs rounded border border-gray-200 p-2 bg-gray-50">
          <p className="font-medium text-gray-700">Status:</p>
          {status.ok ? (
            <div className="space-y-0.5 text-gray-600">
              <p>connected: {String(status.data.connected)}</p>
              <p>agent: {status.data.agentName || '-'}</p>
              <p>inbox: {status.data.inboxUnread ?? 0}/{status.data.inboxTotal ?? 0}</p>
              <p>peers online: {status.data.peersOnline ?? 0}</p>
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
