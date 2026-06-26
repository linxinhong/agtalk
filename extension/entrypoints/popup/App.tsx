import { useState } from 'react';

function App() {
  const [pong, setPong] = useState<boolean | null>(null);

  const pingBackground = async () => {
    const res = await chrome.runtime.sendMessage({ type: 'WXT_PING' });
    setPong(res?.pong ?? false);
  };

  const openApp = () => {
    chrome.tabs.create({ url: chrome.runtime.getURL('/app.html') });
  };

  return (
    <div className="w-72 p-4 space-y-3">
      <h1 className="text-lg font-semibold text-gray-900">agtalk Web Bridge</h1>
      <p className="text-sm text-gray-600">Phase 1 WXT shell</p>
      <div className="flex gap-2">
        <button
          onClick={pingBackground}
          className="flex-1 rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700"
        >
          Ping BG
        </button>
        <button
          onClick={openApp}
          className="flex-1 rounded-md border border-gray-300 px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50"
        >
          Open App
        </button>
      </div>
      {pong !== null && (
        <p className="text-xs text-gray-500">Background: {pong ? 'pong' : 'no response'}</p>
      )}
    </div>
  );
}

export default App;
