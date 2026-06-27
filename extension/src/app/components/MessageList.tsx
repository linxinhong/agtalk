import type { Message } from '@/shared/api/types';
import { useAppStore } from '../store';

export function MessageList() {
  const store = useAppStore();
  const messages = store.messages;
  const selectedMessageId = store.selectedMessageId;

  return (
    <div className="flex-1 overflow-y-auto p-4 space-y-4 bg-gray-50">
      {messages.length === 0 ? (
        <div className="text-center text-sm text-gray-400 py-10">选择会话以查看消息</div>
      ) : (
        messages.map((msg) => (
          <MessageItem
            key={msg.id}
            message={msg}
            selected={msg.id === selectedMessageId}
            onClick={() => store.selectMessage(msg.id)}
          />
        ))
      )}
    </div>
  );
}

function MessageItem({
  message,
  selected,
  onClick,
}: {
  message: Message;
  selected: boolean;
  onClick: () => void;
}) {
  const delivery = message.recipients?.[0];
  const statusText = delivery?.status || message.status || 'pending';
  const isUnread = !delivery?.read_at && (statusText === 'pending' || statusText === 'unread');

  return (
    <button
      onClick={onClick}
      className={`w-full text-left rounded-lg border p-3 transition-colors ${
        selected ? 'border-blue-400 bg-blue-50/40' : 'border-gray-200 bg-white hover:border-gray-300'
      }`}
    >
      <div className="flex items-center justify-between mb-1.5">
        <span className="text-sm font-semibold text-gray-900">{message.sender_name}</span>
        <span className="text-[10px] text-gray-400">{formatTime(message.created_at)}</span>
      </div>
      {message.subject && <p className="text-xs font-medium text-gray-700 mb-1">{message.subject}</p>}
      <p className="text-sm text-gray-700 whitespace-pre-wrap">{message.body}</p>
      <div className="mt-2 flex items-center gap-2 text-[10px] text-gray-500">
        <span className={`w-1.5 h-1.5 rounded-full ${isUnread ? 'bg-blue-500' : 'bg-gray-300'}`} />
        <span>{statusText}</span>
      </div>
    </button>
  );
}

function formatTime(iso: string): string {
  if (!iso) return '-';
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString('zh-CN', { hour12: false });
}
