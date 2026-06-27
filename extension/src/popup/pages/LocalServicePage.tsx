import { useEffect, useState } from 'react';
import { Save, Globe, RefreshCw, Bug } from 'lucide-react';
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
    const trimmed = url.trim();
    const saved = await store.saveConfig({ daemonUrl: trimmed, agtalkUrl: trimmed });
    if (saved) {
      await Promise.all([store.loadHealth(), store.loadStatus()]);
    }
  };

  return (
    <div className="flex flex-col h-full bg-gray-100">
      <Header
        title="本地服务"
        showBack
        onBack={() => store.back()}
        rightActions={
          <>
            <button
              onClick={() => store.loadStatus()}
              disabled={store.loading}
              className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500 disabled:opacity-50"
              title="刷新"
            >
              <RefreshCw size={16} className={store.loading ? 'animate-spin' : ''} />
            </button>
            <button
              onClick={() => store.navigate('debug')}
              className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500"
              title="调试"
            >
              <Bug size={16} />
            </button>
          </>
        }
      />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-3">
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

          <div className="text-xs bg-gray-50 rounded p-2 space-y-1">
            <p className="text-gray-500">
              状态: <span className={store.status?.ok ? 'text-green-600' : 'text-red-500'}>{store.status?.ok ? '在线' : store.status?.error.code || '未连接'}</span>
            </p>
            {store.status?.ok && <p className="text-gray-500">{store.status.data.url}</p>}
          </div>
        </section>

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
