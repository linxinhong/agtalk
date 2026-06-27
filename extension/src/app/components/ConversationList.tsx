import type { Conversation } from '@/shared/api/types';
import { useAppStore } from '../store';
import { RefreshCw } from 'lucide-react';

type FilterStatus = 'all' | 'unread' | 'pending' | 'action_required';

const filters: { key: FilterStatus; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'unread', label: '未读' },
  { key: 'pending', label: '待处理' },
  { key: 'action_required', label: '需操作' },
];

export function ConversationList({ filter, onFilterChange }: { filter: FilterStatus; onFilterChange: (f: FilterStatus) => void }) {
  const store = useAppStore();
  const conversations = store.conversations;
  const selectedId = store.selectedConversationId;

  const filtered = conversations.filter((c) => {
    if (filter === 'all') return true;
    if (filter === 'unread') return (c.counts?.unread ?? 0) > 0;
    if (filter === 'pending') return (c.counts?.pending ?? 0) > 0;
    if (filter === 'action_required') return c.kind === 'action_required';
    return true;
  });

  return (
    <div className="w-80 flex flex-col border-r border-gray-200 bg-white h-full">
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200">
        <div className="flex gap-1">
          {filters.map((f) => (
            <button
              key={f.key}
              onClick={() => onFilterChange(f.key)}
              className={`px-2 py-1 text-[11px] rounded-full border ${
                filter === f.key
                  ? 'bg-blue-600 text-white border-blue-600'
                  : 'border-gray-300 text-gray-600 hover:bg-gray-50'
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>
        <button
          onClick={() => store.loadConversations()}
          disabled={store.loading}
          className="p-1 rounded hover:bg-gray-100 text-gray-500 disabled:opacity-50"
        >
          <RefreshCw size={14} className={store.loading ? 'animate-spin' : ''} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="p-6 text-center text-sm text-gray-400">无会话</div>
        ) : (
          filtered.map((c) => (
            <ConversationItem
              key={c.id}
              conversation={c}
              active={c.id === selectedId}
              onClick={() => store.selectConversation(c.id)}
            />
          ))
        )}
      </div>
    </div>
  );
}

function ConversationItem({
  conversation,
  active,
  onClick,
}: {
  conversation: Conversation;
  active: boolean;
  onClick: () => void;
}) {
  const preview = conversation.last_message?.body || '';
  const title = conversation.title || conversation.peers.join(', ') || '未命名会话';
  const unread = conversation.counts?.unread ?? 0;

  return (
    <button
      onClick={onClick}
      className={`w-full text-left px-3 py-3 border-b border-gray-100 hover:bg-gray-50 ${active ? 'bg-blue-50/60' : ''}`}
    >
      <div className="flex items-center justify-between mb-1">
        <span className="text-sm font-semibold text-gray-900 truncate pr-2">{title}</span>
        {unread > 0 && (
          <span className="shrink-0 px-1.5 py-0.5 text-[10px] bg-blue-600 text-white rounded-full">{unread}</span>
        )}
      </div>
      <p className="text-xs text-gray-500 truncate">{preview || '无消息'}</p>
      <p className="text-[10px] text-gray-400 mt-1">{conversation.peers.join(', ')}</p>
    </button>
  );
}
