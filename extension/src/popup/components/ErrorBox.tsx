import { AlertCircle, X } from 'lucide-react';

interface ErrorBoxProps {
  error: string | null;
  onClose?: () => void;
}

export function ErrorBox({ error, onClose }: ErrorBoxProps) {
  if (!error) return null;
  return (
    <div className="mx-3 mt-2 rounded-md bg-red-50 border border-red-200 p-2 text-xs text-red-700 flex gap-2 items-start">
      <AlertCircle size={14} className="mt-0.5 shrink-0" />
      <span className="flex-1 break-words">{error}</span>
      {onClose && (
        <button onClick={onClose} className="shrink-0 text-red-400 hover:text-red-600">
          <X size={14} />
        </button>
      )}
    </div>
  );
}
