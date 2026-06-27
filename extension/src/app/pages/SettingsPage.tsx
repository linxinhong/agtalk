import { useEffect, useState } from 'react';
import { Save, Server } from 'lucide-react';
import { useAppStore } from '../store';
import { Toggle } from '@/popup/components/Toggle';

export function SettingsPage() {
  const store = useAppStore();
  const config = store.config;

  const [daemonUrl, setDaemonUrl] = useState(config?.daemonUrl || 'http://127.0.0.1:19527');
  const [agentName, setAgentName] = useState(config?.agentName || '');
  const [agentRole, setAgentRole] = useState(config?.agentRole || 'web');
  const [agentBio, setAgentBio] = useState(config?.agentBio || '');
  const [enabled, setEnabled] = useState(config?.enabled ?? true);
  const [autoForward, setAutoForward] = useState(config?.autoForward ?? false);
  const [autoReceive, setAutoReceive] = useState(config?.autoReceive ?? true);
  const [autoInject, setAutoInject] = useState(config?.autoInject ?? false);
  const [chatgpt, setChatgpt] = useState(config?.enableChatgpt ?? true);
  const [claude, setClaude] = useState(config?.enableClaude ?? true);
  const [sider, setSider] = useState(config?.enableSider ?? true);
  const [chatglm, setChatglm] = useState(config?.enableChatglm ?? true);
  const [custom, setCustom] = useState(config?.enableCustom ?? false);
  const [workspaceRoot, setWorkspaceRoot] = useState(config?.workspaceRoot || '/virtual/web-bridge');
  const [workspaceName, setWorkspaceName] = useState(config?.workspaceName || 'web-bridge');
  const [pollInterval, setPollInterval] = useState(config?.pollInterval ?? 5000);
  const [captureDelay, setCaptureDelay] = useState(config?.captureDelay ?? 300);

  useEffect(() => {
    if (config) {
      setDaemonUrl(config.daemonUrl || 'http://127.0.0.1:19527');
      setAgentName(config.agentName || '');
      setAgentRole(config.agentRole || 'web');
      setAgentBio(config.agentBio || '');
      setEnabled(config.enabled ?? true);
      setAutoForward(config.autoForward ?? false);
      setAutoReceive(config.autoReceive ?? true);
      setAutoInject(config.autoInject ?? false);
      setChatgpt(config.enableChatgpt ?? true);
      setClaude(config.enableClaude ?? true);
      setSider(config.enableSider ?? true);
      setChatglm(config.enableChatglm ?? true);
      setCustom(config.enableCustom ?? false);
      setWorkspaceRoot(config.workspaceRoot || '/virtual/web-bridge');
      setWorkspaceName(config.workspaceName || 'web-bridge');
      setPollInterval(config.pollInterval ?? 5000);
      setCaptureDelay(config.captureDelay ?? 300);
    }
  }, [config]);

  const save = async () => {
    if (!agentName.trim()) {
      store.setError('Agent 名称不能为空');
      return;
    }
    await store.saveConfig({
      daemonUrl: daemonUrl.trim(),
      agtalkUrl: daemonUrl.trim(),
      agentName: agentName.trim(),
      agentRole: agentRole.trim() || 'web',
      agentBio: agentBio.trim(),
      enabled,
      autoForward,
      autoReceive,
      autoInject,
      enableChatgpt: chatgpt,
      enableClaude: claude,
      enableSider: sider,
      enableChatglm: chatglm,
      enableCustom: custom,
      workspaceRoot: workspaceRoot.trim(),
      workspaceName: workspaceName.trim(),
      pollInterval: Number(pollInterval) || 5000,
      captureDelay: Number(captureDelay) || 300,
    });
  };

  const Section = ({ title, children }: { title: string; children: React.ReactNode }) => (
    <div className="bg-white rounded-lg border border-gray-200 p-4">
      <h3 className="text-sm font-semibold text-gray-900 mb-3">{title}</h3>
      <div className="space-y-3">{children}</div>
    </div>
  );

  const Field = ({ label, children }: { label: string; children: React.ReactNode }) => (
    <div>
      <label className="block text-xs font-medium text-gray-600 mb-1">{label}</label>
      {children}
    </div>
  );

  return (
    <div className="p-4 space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold text-gray-900">设置</h2>
        <button
          onClick={save}
          disabled={store.loading}
          className="inline-flex items-center gap-1.5 rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
        >
          <Save size={14} />
          保存
        </button>
      </div>

      <Section title="本地服务">
        <Field label="Daemon URL">
          <div className="flex items-center gap-2">
            <Server size={14} className="text-gray-400" />
            <input
              type="text"
              value={daemonUrl}
              onChange={(e) => setDaemonUrl(e.target.value)}
              className="flex-1 rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
            />
          </div>
        </Field>
      </Section>

      <Section title="Agent">
        <Field label="Agent 名称">
          <input
            type="text"
            value={agentName}
            onChange={(e) => setAgentName(e.target.value)}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <Field label="角色">
          <input
            type="text"
            value={agentRole}
            onChange={(e) => setAgentRole(e.target.value)}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <Field label="简介">
          <input
            type="text"
            value={agentBio}
            onChange={(e) => setAgentBio(e.target.value)}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <div className="space-y-1 pt-1">
          <Toggle label="启用 agtalk" checked={enabled} onChange={setEnabled} />
          <Toggle label="自动转发" checked={autoForward} onChange={setAutoForward} />
          <Toggle label="自动接收" checked={autoReceive} onChange={setAutoReceive} />
          <Toggle label="自动注入" checked={autoInject} onChange={setAutoInject} />
        </div>
      </Section>

      <Section title="平台">
        <div className="space-y-1">
          <Toggle label="ChatGPT" checked={chatgpt} onChange={setChatgpt} />
          <Toggle label="Claude" checked={claude} onChange={setClaude} />
          <Toggle label="Sider" checked={sider} onChange={setSider} />
          <Toggle label="ChatGLM" checked={chatglm} onChange={setChatglm} />
          <Toggle label="自定义" checked={custom} onChange={setCustom} />
        </div>
      </Section>

      <Section title="工作区">
        <Field label="Workspace Root">
          <input
            type="text"
            value={workspaceRoot}
            onChange={(e) => setWorkspaceRoot(e.target.value)}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <Field label="Workspace Name">
          <input
            type="text"
            value={workspaceName}
            onChange={(e) => setWorkspaceName(e.target.value)}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <Field label="Poll Interval (ms)">
          <input
            type="number"
            value={pollInterval}
            onChange={(e) => setPollInterval(Number(e.target.value))}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
        <Field label="Capture Delay (ms)">
          <input
            type="number"
            value={captureDelay}
            onChange={(e) => setCaptureDelay(Number(e.target.value))}
            className="w-full rounded-md border border-gray-300 px-2 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
          />
        </Field>
      </Section>
    </div>
  );
}
