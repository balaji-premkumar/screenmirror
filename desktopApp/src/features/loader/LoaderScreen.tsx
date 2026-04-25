import { useState, useEffect, useCallback } from "react";
import { StartupChecks, CheckItem, CheckStatus } from "@/types";
import LogoIcon from "@/components/ui/LogoIcon";

interface LoaderScreenProps {
  onComplete: (checks: StartupChecks) => void;
}

export const LoaderScreen: React.FC<LoaderScreenProps> = ({ onComplete }) => {
  const [checks, setChecks] = useState<CheckItem[]>([
    { label: 'USB Driver Permissions',  status: 'pending', detail: '' },
    { label: 'Native Preview Engine',   status: 'pending', detail: '' },
    { label: 'OBS Studio',              status: 'pending', detail: '' },
    { label: 'OBS Mirror Plugin',        status: 'pending', detail: '' },
  ]);
  const [allDone, setAllDone] = useState(false);
  const [startupData, setStartupData] = useState<StartupChecks | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState(0);

  const getRpc = useCallback(() => {
    // @ts-ignore
    return window.__mirrorRpc || (window.Electrobun && window.Electrobun.rpc);
  }, []);

  const updateCheck = (index: number, status: CheckStatus, detail: string) => {
    setChecks(prev => prev.map((c, i) => i === index ? { ...c, status, detail } : c));
  };

  useEffect(() => {
    let cancelled = false;

    const runChecks = async () => {
      // Delay slightly for the animation to show
      await new Promise(r => setTimeout(r, 600));

      const rpc = getRpc();
      if (!rpc) {
        // RPC not ready, retry after delay
        setTimeout(runChecks, 500);
        return;
      }

      try {
        // Check 1: USB Drivers
        updateCheck(0, 'checking', 'Scanning USB subsystem...');
        await new Promise(r => setTimeout(r, 400));

        const data = await rpc.request('getStartupChecks') as StartupChecks;
        if (cancelled) return;

        setProgress(25);
        updateCheck(0, data.driverOk ? 'ok' : 'warn',
          data.driverOk ? 'Permissions configured' : 'Needs setup — click Fix below');

        // Check 2: Preview Engine
        await new Promise(r => setTimeout(r, 300));
        if (cancelled) return;

        updateCheck(1, 'checking', 'Initializing rendering engine...');
        await new Promise(r => setTimeout(r, 300));
        setProgress(50);
        updateCheck(1, data.ffplayOk ? 'ok' : 'warn',
          data.ffplayOk ? 'Built-in MiniFB engine ready' : 'Preview engine failed to load');

        // Check 3: OBS
        await new Promise(r => setTimeout(r, 300));
        if (cancelled) return;

        updateCheck(2, 'checking', 'Detecting OBS Studio installation...');
        await new Promise(r => setTimeout(r, 400));
        setProgress(75);
        updateCheck(2, data.obsInstalled ? 'ok' : 'warn',
          data.obsInstalled ? 'Installed' : 'Not detected — OBS integration unavailable');

        // Check 4: Plugin
        await new Promise(r => setTimeout(r, 300));
        if (cancelled) return;

        updateCheck(3, 'checking', 'Checking for mirror-source plugin...');
        await new Promise(r => setTimeout(r, 300));
        setProgress(100);

        if (!data.obsInstalled) {
          updateCheck(3, 'warn', 'Skipped — OBS not installed');
        } else {
          updateCheck(3, data.obsPluginInstalled ? 'ok' : 'warn',
            data.obsPluginInstalled
              ? `Installed at ${data.obsPluginDir}`
              : 'Not installed — click Install below');
        }

        setStartupData(data);
        setAllDone(true);

      } catch (e) {
        console.error('Startup checks failed:', e);
        // Mark all as warn and continue
        for (let i = 0; i < 4; i++) {
          updateCheck(i, 'warn', 'Check failed — continuing');
        }
        setStartupData({ driverOk: false, ffplayOk: false, obsInstalled: false, obsPluginInstalled: false, obsPluginDir: '' });
        setAllDone(true);
        setProgress(100);
      }
    };

    runChecks();
    return () => { cancelled = true; };
  }, [getRpc]);

   const installPlugin = async () => {
     setInstalling(true);
     try {
       const rpc = getRpc();
       if (rpc) {
         const res = await rpc.request('installObsPlugin') as { success: boolean; error?: string };
         if (res.success) {
           updateCheck(3, 'ok', 'Plugin installed — restart OBS to activate');
           if (startupData) setStartupData({ ...startupData, obsPluginInstalled: true });
         } else {
           const errorMsg = res.error || 'Unknown error occurred during installation';
           updateCheck(3, 'error', `Installation failed: ${errorMsg}`);
         }
       }
     } catch (e: any) {
       const errorMessage = e.message || String(e) || 'Unknown error';
       updateCheck(3, 'error', `Installation error: ${errorMessage}`);
     } finally {
       setInstalling(false);
     }
   };

  const getStatusIcon = (s: CheckStatus) => {
    switch (s) {
      case 'pending': return <div className="w-4 h-4 rounded-full border-2 border-gray-700" />;
      case 'checking': return <div className="w-4 h-4 rounded-full border-2 border-orange-400 border-t-transparent animate-spin" />;
      case 'ok': return <div className="w-4 h-4 rounded-full bg-green-500 flex items-center justify-center text-[8px] text-black font-black">✓</div>;
      case 'warn': return <div className="w-4 h-4 rounded-full bg-yellow-500/80 flex items-center justify-center text-[8px] text-black font-black">!</div>;
      case 'error': return <div className="w-4 h-4 rounded-full bg-red-500 flex items-center justify-center text-[8px] text-white font-black">✕</div>;
    }
  };

  const getStatusColor = (s: CheckStatus) => {
    switch (s) {
      case 'pending': return 'text-gray-600';
      case 'checking': return 'text-orange-400';
      case 'ok': return 'text-green-400';
      case 'warn': return 'text-yellow-400';
      case 'error': return 'text-red-400';
    }
  };

  return (
    <div className="min-h-screen bg-[#050608] flex items-center justify-center p-6">
      <div className="w-full max-w-lg">
        {/* Logo & Title */}
        <div className="text-center mb-10">
          <div className="inline-flex items-center justify-center mb-6">
            <div className="relative">
              <div className="absolute inset-0 blur-2xl bg-orange-500/20 rounded-full animate-pulse" />
              <LogoIcon />
            </div>
          </div>
          <h1 className="text-3xl font-black tracking-tighter text-transparent bg-clip-text bg-gradient-to-r from-orange-400 to-orange-600 uppercase mb-2">
            Mirror Core
          </h1>
          <p className="text-[10px] text-gray-500 font-mono tracking-[0.4em] uppercase">
            Initializing Enterprise Platform
          </p>
        </div>

        {/* Progress Bar */}
        <div className="mb-8 px-4">
          <div className="h-[2px] bg-gray-800 rounded-full overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-orange-500 to-orange-400 rounded-full transition-all duration-700 ease-out"
              style={{ width: `${progress}%` }}
            />
          </div>
        </div>

        {/* Check Items */}
        <div className="bg-[#0a0c10] rounded-2xl border border-gray-800/50 p-6 space-y-1 mb-6 shadow-2xl">
          {checks.map((check, i) => (
            <div key={i} className={`flex items-start gap-4 py-3 px-3 rounded-xl transition-all duration-300 ${check.status === 'checking' ? 'bg-orange-500/5' : ''}`}>
              <div className="mt-0.5 flex-shrink-0">
                {getStatusIcon(check.status)}
              </div>
              <div className="flex-1 min-w-0">
                <div className={`text-xs font-bold ${check.status === 'pending' ? 'text-gray-600' : 'text-gray-200'} transition-colors duration-300`}>
                  {check.label}
                </div>
                {check.detail && (
                  <div className={`text-[10px] mt-0.5 ${getStatusColor(check.status)} font-medium truncate`}>
                    {check.detail}
                  </div>
                )}
              </div>
              {/* Install button for OBS plugin */}
              {i === 3 && check.status === 'warn' && startupData?.obsInstalled && !startupData?.obsPluginInstalled && (
                <button
                  onClick={installPlugin}
                  disabled={installing}
                  className="text-[9px] font-black uppercase px-3 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/20 text-blue-400 hover:bg-blue-500/20 transition-all cursor-pointer flex-shrink-0"
                >
                  {installing ? 'Installing...' : 'Install'}
                </button>
              )}
            </div>
          ))}
        </div>

        {/* Enter Button */}
        <div className="flex justify-center">
          <button
            onClick={() => startupData && onComplete(startupData)}
            disabled={!allDone}
            className={`px-12 py-3 rounded-xl font-black text-xs uppercase tracking-widest transition-all duration-500 cursor-pointer ${
              allDone
                ? 'bg-gradient-to-r from-orange-500 to-orange-600 text-black shadow-xl shadow-orange-900/30 hover:shadow-orange-600/40 hover:scale-[1.02] active:scale-[0.98]'
                : 'bg-gray-800 text-gray-600 cursor-not-allowed'
            }`}
          >
            {allDone ? 'Enter Dashboard' : 'System Check in Progress...'}
          </button>
        </div>

        {/* Version footer */}
        <p className="text-center text-[9px] text-gray-700 mt-8 tracking-widest uppercase">
          v1.0 Enterprise • USB AV Pipeline
        </p>
      </div>
    </div>
  );
};