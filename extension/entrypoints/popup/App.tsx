import { usePopupStore, type PopupPage, type PopupState, type PopupActions } from '@/popup/store';
import { HomePage } from '@/popup/pages/HomePage';
import { AgentsPage } from '@/popup/pages/AgentsPage';
import { AgentConfigPage } from '@/popup/pages/AgentConfigPage';
import { LocalServicePage } from '@/popup/pages/LocalServicePage';
import { PlatformConfigPage } from '@/popup/pages/PlatformConfigPage';
import { DebugPage } from '@/popup/pages/DebugPage';

function App() {
  const page = usePopupStore((s: PopupState & PopupActions) => s.page);

  return (
    <div className="w-[360px] h-[540px] bg-gray-100 text-gray-900 font-sans overflow-hidden shadow-xl">
      <Page page={page} />
    </div>
  );
}

function Page({ page }: { page: PopupPage }) {
  switch (page) {
    case 'agents':
      return <AgentsPage />;
    case 'agentConfig':
      return <AgentConfigPage />;
    case 'localService':
      return <LocalServicePage />;
    case 'platformConfig':
      return <PlatformConfigPage />;
    case 'debug':
      return <DebugPage />;
    case 'home':
    default:
      return <HomePage />;
  }
}

export default App;
