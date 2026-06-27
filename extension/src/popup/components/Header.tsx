import { ArrowLeft, X } from 'lucide-react';

interface HeaderProps {
  title: string;
  showBack?: boolean;
  onBack?: () => void;
}

export function Header({ title, showBack, onBack }: HeaderProps) {
  return (
    <header className="flex items-center justify-between px-3 py-2.5 bg-white border-b border-gray-200">
      <div className="flex items-center gap-2">
        {showBack ? (
          <button
            onClick={onBack}
            className="p-1 rounded hover:bg-gray-100 text-gray-600"
            aria-label="返回"
          >
            <ArrowLeft size={16} />
          </button>
        ) : (
          <span className="w-6" />
        )}
        <h1 className="text-base font-semibold text-gray-900">{title}</h1>
      </div>
      <button
        onClick={() => window.close()}
        className="p-1 rounded hover:bg-gray-100 text-gray-500"
        aria-label="关闭"
      >
        <X size={16} />
      </button>
    </header>
  );
}
