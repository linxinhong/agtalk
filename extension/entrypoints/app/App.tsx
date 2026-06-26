import { useState } from 'react';
import { Activity } from 'lucide-react';

function App() {
  const [pong, setPong] = useState<boolean | null>(null);

  const pingBackground = async () => {
    const res = await chrome.runtime.sendMessage({ type: 'PING_BACKGROUND' });
    setPong(res?.pong ?? false);
  };

  return (
    <div className="min-h-screen p-8 space-y-4">
      <h1 className="text-2xl font-bold text-gray-900">agtalk App</h1>
      <p className="text-gray-600">Phase 1 WXT shell</p>
      <button
        onClick={pingBackground}
        className="inline-flex items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700"
      >
        <Activity size={16} />
        Ping Background
      </button>
      {pong !== null && (
        <p className="text-sm text-gray-500">Background: {pong ? 'pong' : 'no response'}</p>
      )}
    </div>
  );
}

export default App;
