import { useState } from 'react';
import { Dashboard } from '@/features/dashboard/Dashboard';
import { LoaderScreen } from '@/features/loader/LoaderScreen';
import { I18nProvider } from '@/i18n';
import type { StartupChecks } from '@/types';

/**
 * The startup checks gate the dashboard: it needs to know whether ffplay and
 * the OBS plugin exist before it can decide which sink controls to offer, and
 * asking again on every render would be four subprocess spawns a second.
 */
function App() {
  const [startupChecks, setStartupChecks] = useState<StartupChecks | null>(null);

  return (
    <I18nProvider>
      {startupChecks ? (
        <Dashboard startupChecks={startupChecks} />
      ) : (
        <LoaderScreen onComplete={setStartupChecks} />
      )}
    </I18nProvider>
  );
}

export default App;
