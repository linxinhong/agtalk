import { useState } from 'react';
import { ConversationList } from '../components/ConversationList';
import { MessageList } from '../components/MessageList';
import { ReplyBox } from '../components/ReplyBox';
import { useAppStore } from '../store';

type FilterStatus = 'all' | 'unread' | 'pending' | 'action_required';

export function ConversationsPage() {
  const store = useAppStore();
  const [filter, setFilter] = useState<FilterStatus>('all');

  const offline = store.health?.ok === false;

  return (
    <div className="flex flex-col h-full">
      {offline && (
        <div className="px-4 py-2 bg-orange-50 border-b border-orange-200 text-sm text-orange-700">
          本地 Agent 服务未连接，请确认 http://127.0.0.1:19527 已启动。
        </div>
      )}
      <div className="flex flex-1 overflow-hidden">
        <ConversationList filter={filter} onFilterChange={setFilter} />
        <div className="flex flex-col flex-1 min-w-0">
          <MessageList />
          <ReplyBox />
        </div>
      </div>
    </div>
  );
}
