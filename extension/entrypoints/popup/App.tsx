import { usePopupStore } from '@/popup/store';
import { HomePage } from '@/popup/pages/HomePage';
import { AgentsPage } from '@/popup/pages/AgentsPage';
import { AgentConfigPage } from '@/popup/pages/AgentConfigPage';
import { LocalServicePage } from '@/popup/pages/LocalServicePage';
import { PlatformConfigPage } from '@/popup/pages/PlatformConfigPage';
import { DebugPage } from '@/popup/pages/DebugPage';

function App() {
  const page = usePopupStore((s) => s.page);

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
