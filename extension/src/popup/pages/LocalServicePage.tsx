import { useEffect, useState } from 'react';
import { Save, Globe } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { usePopupStore } from '../store';

export function LocalServicePage() {
  const store = usePopupStore();
  const config = store.config;
  const [url, setUrl] = useState(config?.daemonUrl || 'http://127.0.0.1:19527');

  useEffect(() => {
    if (config?.daemonUrl) setUrl(config.daemonUrl);
  }, [config?.daemonUrl]);

  const save = async () => {
    const saved = await store.saveConfig({ daemonUrl: url.trim(), agtalkUrl: url.trim() });
    if (saved) {
      await store.loadStatus();
    }
  };

  return (
    <div className="flex flex-col h-full bg-gray-50">
      <Header title="本地服务" showBack onBack={() => store.back()} />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="bg-white rounded-lg border border-gray-200 p-3 space-y-3">
          <div>
            <label className="block text-xs font-medium text-gray-600 mb-1">Daemon URL</label>
            <div className="flex items-center gap-2">
              <Globe size={14} className="text-gray-400" />
              <input
                type="text"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                className="flex-1 rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
              />
            </div>
          </div>

          <div className="text-xs text-gray-500 bg-gray-50 rounded p-2">
            <p>当前状态: {store.status?.ok ? '在线' : store.status?.error.code || '未连接'}</p>
            {store.status?.ok && <p className="mt-1">{store.status.data.url}</p>}
          </div>
        </div>

        <button
          onClick={save}
          disabled={store.loading}
          className="w-full inline-flex items-center justify-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
        >
          <Save size={14} />
          保存并刷新状态
        </button>
      </div>
    </div>
  );
}
