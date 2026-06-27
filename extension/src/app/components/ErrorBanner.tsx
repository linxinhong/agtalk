import { X, AlertCircle, CheckCircle2 } from 'lucide-react';
import { useAppStore } from '../store';

export function ErrorBanner() {
  const store = useAppStore();
  const error = store.error;
  const success = store.success;

  return (
    <>
      {error && (
        <div className="flex items-start gap-2 px-4 py-2.5 bg-red-50 border-b border-red-200 text-sm text-red-700">
          <AlertCircle size={16} className="mt-0.5 shrink-0" />
          <span className="flex-1 break-words">{error}</span>
          <button onClick={() => store.clearError()} className="shrink-0 text-red-400 hover:text-red-600">
            <X size={16} />
          </button>
        </div>
      )}
      {success && (
        <div className="flex items-start gap-2 px-4 py-2.5 bg-green-50 border-b border-green-200 text-sm text-green-700">
          <CheckCircle2 size={16} className="mt-0.5 shrink-0" />
          <span className="flex-1 break-words">{success}</span>
          <button onClick={() => store.clearSuccess()} className="shrink-0 text-green-500 hover:text-green-700">
            <X size={16} />
          </button>
        </div>
      )}
    </>
  );
}
