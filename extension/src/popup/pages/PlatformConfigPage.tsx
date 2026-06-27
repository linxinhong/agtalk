import { useEffect, useState } from 'react';
import { Save, Monitor } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { Toggle } from '../components/Toggle';
import { usePopupStore } from '../store';

export function PlatformConfigPage() {
  const store = usePopupStore();
  const config = store.config;

  const [chatgpt, setChatgpt] = useState(config?.enableChatgpt ?? true);
  const [claude, setClaude] = useState(config?.enableClaude ?? true);
  const [sider, setSider] = useState(config?.enableSider ?? true);
  const [chatglm, setChatglm] = useState(config?.enableChatglm ?? true);
  const [custom, setCustom] = useState(config?.enableCustom ?? false);

  useEffect(() => {
    if (config) {
      setChatgpt(config.enableChatgpt ?? true);
      setClaude(config.enableClaude ?? true);
      setSider(config.enableSider ?? true);
      setChatglm(config.enableChatglm ?? true);
      setCustom(config.enableCustom ?? false);
    }
  }, [config]);

  const save = async () => {
    await store.saveConfig({
      enableChatgpt: chatgpt,
      enableClaude: claude,
      enableSider: sider,
      enableChatglm: chatglm,
      enableCustom: custom,
    });
  };

  return (
    <div className="flex flex-col h-full bg-gray-50">
      <Header title="平台开关" showBack onBack={() => store.back()} />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="bg-white rounded-lg border border-gray-200 p-3">
          <div className="flex items-center gap-2 mb-2">
            <Monitor size={16} className="text-gray-500" />
            <span className="text-sm font-medium text-gray-700">注入目标平台</span>
          </div>
          <div className="space-y-1">
            <Toggle label="ChatGPT" checked={chatgpt} onChange={setChatgpt} />
            <Toggle label="Claude" checked={claude} onChange={setClaude} />
            <Toggle label="Sider" checked={sider} onChange={setSider} />
            <Toggle label="ChatGLM" checked={chatglm} onChange={setChatglm} />
            <Toggle label="自定义" checked={custom} onChange={setCustom} />
          </div>
        </div>

        <button
          onClick={save}
          disabled={store.loading}
          className="w-full inline-flex items-center justify-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
        >
          <Save size={14} />
          保存平台配置
        </button>
      </div>
    </div>
  );
}
