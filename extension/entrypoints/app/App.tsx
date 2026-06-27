import { useEffect } from 'react';
import { Sidebar } from '@/app/components/Sidebar';
import { ErrorBanner } from '@/app/components/ErrorBanner';
import { ConversationsPage } from '@/app/pages/ConversationsPage';
import { SettingsPage } from '@/app/pages/SettingsPage';
import { LogsPage } from '@/app/pages/LogsPage';
import { StatusPage } from '@/app/pages/StatusPage';
import { useAppStore, type AppPage } from '@/app/store';

function App() {
  const { activePage } = useAppStore();

  useEffect(() => {
    useAppStore.getState().bootstrap();
  }, []);

  return (
    <div className="min-h-screen bg-gray-100 text-gray-900 font-sans flex">
      <Sidebar />
      <div className="flex-1 flex flex-col min-w-0">
        <ErrorBanner />
        <main className="flex-1 overflow-hidden">
          <Page page={activePage} />
        </main>
      </div>
    </div>
  );
}

function Page({ page }: { page: AppPage }) {
  switch (page) {
    case 'settings':
      return <SettingsPage />;
    case 'logs':
      return <LogsPage />;
    case 'status':
      return <StatusPage />;
    case 'conversations':
    default:
      return <ConversationsPage />;
  }
}

export default App;
