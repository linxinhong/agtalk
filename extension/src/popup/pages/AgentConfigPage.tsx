import { useEffect, useState } from 'react';
import { Save, User } from 'lucide-react';
import { Header } from '../components/Header';
import { ErrorBox } from '../components/ErrorBox';
import { Toggle } from '../components/Toggle';
import { usePopupStore } from '../store';

export function AgentConfigPage() {
  const store = usePopupStore();
  const config = store.config;

  const [agentName, setAgentName] = useState(config?.agentName || '');
  const [agentRole, setAgentRole] = useState(config?.agentRole || 'web');
  const [agentBio, setAgentBio] = useState(config?.agentBio || '');
  const [enabled, setEnabled] = useState(config?.enabled ?? true);
  const [autoForward, setAutoForward] = useState(config?.autoForward ?? false);
  const [autoReceive, setAutoReceive] = useState(config?.autoReceive ?? true);

  useEffect(() => {
    if (config) {
      setAgentName(config.agentName || '');
      setAgentRole(config.agentRole || 'web');
      setAgentBio(config.agentBio || '');
      setEnabled(config.enabled ?? true);
      setAutoForward(config.autoForward ?? false);
      setAutoReceive(config.autoReceive ?? true);
    }
  }, [config]);

  const save = async () => {
    const saved = await store.saveConfig({
      agentName: agentName.trim(),
      agentRole: agentRole.trim() || 'web',
      agentBio: agentBio.trim(),
      enabled,
      autoForward,
      autoReceive,
    });
    if (saved && saved.agentName) {
      await store.registerAgent();
    }
  };

  return (
    <div className="flex flex-col h-full bg-gray-50">
      <Header title="Agent 配置" showBack onBack={() => store.back()} />
      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <div className="bg-white rounded-lg border border-gray-200 p-3 space-y-3">
          <div>
            <label className="block text-xs font-medium text-gray-600 mb-1">Agent 名称</label>
            <div className="flex items-center gap-2">
              <User size={14} className="text-gray-400" />
              <input
                type="text"
                value={agentName}
                onChange={(e) => setAgentName(e.target.value)}
                placeholder="web_chatgpt_Test"
                className="flex-1 rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-medium text-gray-600 mb-1">角色</label>
            <input
              type="text"
              value={agentRole}
              onChange={(e) => setAgentRole(e.target.value)}
              className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-gray-600 mb-1">简介</label>
            <input
              type="text"
              value={agentBio}
              onChange={(e) => setAgentBio(e.target.value)}
              placeholder="Web AI bridge participant"
              className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
            />
          </div>

          <div className="space-y-1">
            <Toggle label="启用 agtalk" checked={enabled} onChange={setEnabled} />
            <Toggle label="自动转发" checked={autoForward} onChange={setAutoForward} />
            <Toggle label="自动接收" checked={autoReceive} onChange={setAutoReceive} />
          </div>
        </div>

        <button
          onClick={save}
          disabled={store.loading || !agentName.trim()}
          className="w-full inline-flex items-center justify-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 disabled:bg-gray-400"
        >
          <Save size={14} />
          保存并 Join
        </button>
      </div>
    </div>
  );
}
