import { useState } from 'react';
import { Send } from 'lucide-react';
import { useAppStore } from '../store';

export function ReplyBox() {
  const store = useAppStore();
  const [text, setText] = useState('');

  const handleSend = async () => {
    if (!text.trim()) return;
    const ok = await store.sendReply(text);
    if (ok) {
      setText('');
    }
  };

  return (
    <div className="border-t border-gray-200 bg-white p-3">
      <div className="flex gap-2">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={store.selectedMessageId ? '回复选中消息...' : '选择一条消息后回复'}
          disabled={!store.selectedMessageId || store.loading}
          className="flex-1 min-h-[64px] max-h-32 rounded-md border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none resize-y disabled:bg-gray-50"
        />
        <button
          onClick={handleSend}
          disabled={!store.selectedMessageId || store.loading || !text.trim()}
          className="self-end inline-flex items-center gap-1.5 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 disabled:bg-gray-400"
        >
          <Send size={14} />
          发送
        </button>
      </div>
    </div>
  );
}
