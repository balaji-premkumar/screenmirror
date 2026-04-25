import { useState } from "react";
import { StartupChecks } from "@/types";
import { LoaderScreen } from "@/features/loader/LoaderScreen";
import { Dashboard } from "@/features/dashboard/Dashboard";

function App() {
  const [phase, setPhase] = useState<'loading' | 'dashboard'>('loading');
  const [startupChecks, setStartupChecks] = useState<StartupChecks | null>(null);

  const handleLoaderComplete = (checks: StartupChecks) => {
    setStartupChecks(checks);
    setPhase('dashboard');
  };

  if (phase === 'loading') {
    return <LoaderScreen onComplete={handleLoaderComplete} />;
  }

  return <Dashboard startupChecks={startupChecks!} />;
}

export default App;
