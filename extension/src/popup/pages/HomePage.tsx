import { useEffect } from 'react';
import {
  RefreshCw,
  Users,
  Settings,
  Layers,
  Bug,
  ExternalLink,
  Zap,
  Unplug,
  ChevronRight,
  Mail,
  Bot,
} from 'lucide-react';
import { usePopupStore, type PopupPage } from '../store';
import { Toggle } from '../components/Toggle';
import { ErrorBox } from '../components/ErrorBox';
import { MessageType } from '@/shared/messaging/message-types';
import { sendMessage } from '@/shared/messaging/send-message';

export function HomePage() {
  const store = usePopupStore();
  const config = store.config;
  const status = store.status;
  const peers = store.peers;
  const inbox = store.inbox;

  const connected = status?.ok && status.data.connected;
  const agentName = config?.agentName || '';
  const activePeer = config?.activePeer || config?.targetAgent || '';
  const connectedCount = config?.connectedPeers?.length ?? 0;

  useEffect(() => {
    store.loadAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openApp = async () => {
    await sendMessage({ type: MessageType.OPEN_APP_PAGE });
  };

  const statusDotClass = connected
    ? 'bg-green-600'
    : status?.ok
      ? 'bg-yellow-500'
      : 'bg-red-500';
  const statusLabel = connected ? '在线' : status?.ok ? '异常' : '离线';

  return (
    <div className="flex flex-col h-full bg-gray-100">
      {/* A. Header */}
      <header className="flex items-center justify-between px-3 py-2.5 bg-white border-b border-gray-200">
        <div>
          <h1 className="text-base font-bold text-gray-900 leading-tight">agtalk</h1>
          <p className="text-[10px] text-gray-500 leading-tight">Web Bridge</p>
        </div>
        <div className="flex items-center gap-1.5">
          <div className="flex items-center gap-1.5 px-2 py-1 rounded-full bg-gray-50 border border-gray-200">
            <span className={`w-2 h-2 rounded-full ${statusDotClass}`} />
            <span className="text-[11px] text-gray-600">{statusLabel}</span>
          </div>
          <button
            onClick={() => store.loadAll()}
            disabled={store.loading}
            className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500 disabled:opacity-50"
            title="刷新"
          >
            <RefreshCw size={15} className={store.loading ? 'animate-spin' : ''} />
          </button>
          <button
            onClick={openApp}
            className="p-1.5 rounded-md hover:bg-gray-100 text-gray-500"
            title="打开 App"
          >
            <ExternalLink size={15} />
          </button>
        </div>
      </header>

      <ErrorBox error={store.lastError} onClose={() => store.setLastError(null)} />

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {/* B. 状态条 */}
        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-2">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Bot size={16} className="text-gray-400" />
              <span className="text-sm font-medium text-gray-900 truncate max-w-[180px]">
                {agentName || '未配置 Agent'}
              </span>
            </div>
            <span className={`text-xs font-medium ${connected ? 'text-green-600' : 'text-red-500'}`}>
              {statusLabel}
            </span>
          </div>
          <div className="flex justify-between text-xs text-gray-500">
            <span>inbox {status?.ok ? `${status.data.inboxUnread ?? 0}/${status.data.inboxTotal ?? 0}` : '-'}</span>
            <span>peers {status?.ok ? status.data.peersOnline ?? 0 : '-'}</span>
          </div>
          {status?.ok && (
            <div className="space-y-1">
              {status.data.authError && <p className="text-[11px] text-orange-600 bg-orange-50 rounded px-2 py-1">auth: {status.data.authError}</p>}
              {status.data.inboxError && <p className="text-[11px] text-orange-600 bg-orange-50 rounded px-2 py-1">inbox: {status.data.inboxError}</p>}
              {status.data.peersError && <p className="text-[11px] text-orange-600 bg-orange-50 rounded px-2 py-1">peers: {status.data.peersError}</p>}
            </div>
          )}
        </section>

        {/* C. 快速动作区 */}
        <section className="grid grid-cols-3 gap-2">
          <ActionButton
            icon={Zap}
            label="Join"
            onClick={() => store.registerAgent()}
            disabled={store.loading || !agentName}
          />
          <ActionButton
            icon={Unplug}
            label="重连"
            onClick={() => store.reconnect()}
            disabled={store.loading || !agentName}
          />
          <ActionButton
            icon={RefreshCw}
            label="刷新"
            onClick={() => store.loadAll()}
            disabled={store.loading}
          />
        </section>
        {!agentName && (
          <p className="text-[11px] text-orange-600 bg-orange-50 border border-orange-100 rounded-md px-2 py-1.5">
            请在「Agent 配置」中设置 agentName 后再 Join。
          </p>
        )}

        {/* D. 自动注入总开关 */}
        <section className="bg-white rounded-lg border border-gray-200 p-3">
          <Toggle
            label="自动注入新消息到对话"
            checked={!!config?.autoInject}
            onChange={(v) => store.setAutoInject(v)}
            disabled={store.loading}
          />
          <p className="text-[11px] text-gray-400 mt-1.5">
            关闭时会清空 peer 级自动注入列表
          </p>
        </section>

        {/* E. 目标 Agent / Peer 摘要 */}
        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-gray-600 uppercase tracking-wide">目标 Agent</span>
            <button
              onClick={() => store.navigate('agents')}
              className="flex items-center text-[11px] text-blue-600 hover:text-blue-700"
            >
              管理 <ChevronRight size={12} />
            </button>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-gray-500">active / target</span>
            <span className="font-medium text-gray-900 truncate max-w-[180px]">
              {activePeer || '-'}
            </span>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-gray-500">已连接</span>
            <span className="font-medium text-gray-900">{connectedCount} 个</span>
          </div>
          {connectedCount > 0 && (
            <div className="flex flex-wrap gap-1">
              {config?.connectedPeers?.slice(0, 5).map((name) => (
                <span key={name} className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 border border-gray-200 truncate max-w-[100px]">
                  {name}
                </span>
              ))}
            </div>
          )}
        </section>

        {/* F. Inbox 摘要 */}
        <section className="bg-white rounded-lg border border-gray-200 p-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-gray-600 uppercase tracking-wide">Inbox 摘要</span>
            <div className="flex items-center gap-1 text-[11px] text-gray-500">
              <Mail size={12} />
              <span>{inbox ? `${inbox.unread}/${inbox.total}` : '-'}</span>
            </div>
          </div>

          {inbox?.migrationPending ? (
            <p className="text-xs text-orange-600 bg-orange-50 rounded px-2 py-2">Inbox 迁移中</p>
          ) : inbox && inbox.items.length > 0 ? (
            <div className="space-y-2">
              {inbox.items.slice(0, 5).map((item) => {
                const body = item.body || '';
                const short = body.length > 60 ? body.slice(0, 60) + '…' : body;
                const isUnread = !item.read_at && item.status !== 'read';
                return (
                  <div
                    key={item.id}
                    className={`text-xs border-l-2 pl-2 py-1 ${isUnread ? 'border-blue-500 bg-blue-50/50' : 'border-gray-200'}`}
                  >
                    <div className="flex items-center justify-between mb-0.5">
                      <span className="font-medium text-gray-800 truncate max-w-[180px]">{item.from_name}</span>
                      {isUnread && <span className="w-1.5 h-1.5 rounded-full bg-blue-500" />}
                    </div>
                    <p className="text-gray-500 truncate">{short}</p>
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-xs text-gray-400 py-2 text-center">暂无消息</p>
          )}
        </section>
      </div>

      {/* G. 二级入口 */}
      <nav className="flex items-center justify-around bg-white border-t border-gray-200 py-2">
        <NavButton icon={Users} label="Agents" page="agents" />
        <NavButton icon={Settings} label="Agent" page="agentConfig" />
        <NavButton icon={Layers} label="Platforms" page="platformConfig" />
        <NavButton icon={Bug} label="Debug" page="debug" />
      </nav>
    </div>
  );
}

function ActionButton({
  icon: Icon,
  label,
  onClick,
  disabled,
}: {
  icon: React.ElementType;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex flex-col items-center justify-center gap-1 rounded-lg border border-gray-200 bg-white py-2.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 disabled:bg-gray-100"
    >
      <Icon size={16} />
      {label}
    </button>
  );
}

function NavButton({
  icon: Icon,
  label,
  page,
}: {
  icon: React.ElementType;
  label: string;
  page: PopupPage;
}) {
  const store = usePopupStore();
  return (
    <button
      onClick={() => store.navigate(page)}
      className="flex flex-col items-center gap-0.5 text-[10px] text-gray-600 hover:text-blue-600"
    >
      <Icon size={16} />
      {label}
    </button>
  );
}
