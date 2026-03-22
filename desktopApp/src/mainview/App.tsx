import { useState, useEffect, useRef, memo, useCallback } from "react";

// ── Types ────────────────────────────────────────────────────

interface LogEntry {
  timestamp: string;
  level: string;
  module: string;
  thread: string;
  message: string;
}

interface Metrics {
    throughput_mbps: number;
    pipeline_latency_ms: number;
    fps_actual: number;
    frames_dropped: number;
    buffer_health: number;
}

interface StatusUpdate {
  bufferSize: number;
  isActive: boolean;
  decoder: string;
  devices: string[];
  newLogs: LogEntry[];
  metrics: Metrics;
  driverOk: boolean;
}

interface StartupChecks {
  driverOk: boolean;
  ffplayOk: boolean;
  obsInstalled: boolean;
  obsPluginInstalled: boolean;
  obsPluginDir: string;
}

declare global {
  interface Window {
    __mirrorRpc: {
        request: (method: string, data?: any) => Promise<any>;
    };
    Electrobun: {
        rpc: {
            request: (method: string, data?: any) => Promise<any>;
        }
    }
  }
}

const MAX_LOGS = 300;

// ── Logo SVG (inline) ────────────────────────────────────────

const LogoIcon = () => (
  <svg width="40" height="40" viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
    <rect x="2" y="2" width="36" height="36" rx="8" stroke="url(#logoGrad)" strokeWidth="2.5" fill="none"/>
    <path d="M12 14 L20 10 L28 14 L28 26 L20 30 L12 26 Z" fill="url(#logoGrad)" opacity="0.15"/>
    <path d="M12 14 L20 10 L28 14 M12 14 L20 18 L28 14 M20 18 L20 30" stroke="url(#logoGrad)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
    <defs>
      <linearGradient id="logoGrad" x1="0" y1="0" x2="40" y2="40" gradientUnits="userSpaceOnUse">
        <stop stopColor="#fb923c"/>
        <stop offset="1" stopColor="#ea580c"/>
      </linearGradient>
    </defs>
  </svg>
);

// ── Check item statuses ──────────────────────────────────────

type CheckStatus = 'pending' | 'checking' | 'ok' | 'warn' | 'error';

interface CheckItem {
  label: string;
  status: CheckStatus;
  detail: string;
}

// ═══════════════════════════════════════════════════════════
//  LOADER SCREEN
// ═══════════════════════════════════════════════════════════

function LoaderScreen({ onComplete }: { onComplete: (checks: StartupChecks) => void }) {
  const [checks, setChecks] = useState<CheckItem[]>([
    { label: 'USB Driver Permissions',  status: 'pending', detail: '' },
    { label: 'ffplay (Video Preview)',   status: 'pending', detail: '' },
    { label: 'OBS Studio',              status: 'pending', detail: '' },
    { label: 'OBS Mirror Plugin',        status: 'pending', detail: '' },
  ]);
  const [allDone, setAllDone] = useState(false);
  const [startupData, setStartupData] = useState<StartupChecks | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState(0);

  const getRpc = useCallback(() => {
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

        // Check 2: ffplay
        await new Promise(r => setTimeout(r, 300));
        if (cancelled) return;

        updateCheck(1, 'checking', 'Looking for ffplay binary...');
        await new Promise(r => setTimeout(r, 300));
        setProgress(50);
        updateCheck(1, data.ffplayOk ? 'ok' : 'warn',
          data.ffplayOk ? 'Available on PATH' : 'Not found — install ffmpeg for video preview');

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
        const res = await rpc.request('installObsPlugin') as { success: boolean };
        if (res.success) {
          updateCheck(3, 'ok', 'Plugin installed — restart OBS to activate');
          if (startupData) setStartupData({ ...startupData, obsPluginInstalled: true });
        } else {
          updateCheck(3, 'error', 'Installation failed — check logs');
        }
      }
    } catch (e) {
      updateCheck(3, 'error', 'Installation error');
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
}

// ═══════════════════════════════════════════════════════════
//  LOG ITEM (memoised)
// ═══════════════════════════════════════════════════════════

const LogItem = memo(({ log }: { log: LogEntry }) => {
    const getLevelColor = (level: string) => {
        switch (level) {
            case 'ERROR': case 'FATAL': return 'text-red-400';
            case 'WARN': return 'text-yellow-400';
            case 'SUCCESS': return 'text-green-400';
            default: return 'text-blue-400';
        }
    };
    return (
        <div className="border-l border-gray-800 pl-3 py-0.5 hover:bg-white/5 text-[10px] font-mono flex gap-2">
            <span className="text-gray-600 min-w-[80px]">{log.timestamp}</span>
            <span className={`${getLevelColor(log.level)} font-bold w-[70px]`}>[{log.level}]</span>
            <span className="text-gray-300 break-all">{log.message}</span>
        </div>
    );
});

// ═══════════════════════════════════════════════════════════
//  MAIN DASHBOARD
// ═══════════════════════════════════════════════════════════

function Dashboard({ startupChecks }: { startupChecks: StartupChecks }) {
  const [status, setStatus] = useState<{
    decoder: string;
    isConnected: boolean;
    bufferSize: number;
    devices: string[];
    logs: LogEntry[];
    metrics: Metrics;
    driverOk: boolean;
  }>({
    decoder: "Initializing...",
    isConnected: false,
    bufferSize: 0,
    devices: [],
    logs: [],
    metrics: { throughput_mbps: 0, pipeline_latency_ms: 0, fps_actual: 0, frames_dropped: 0, buffer_health: 0 },
    driverOk: startupChecks.driverOk
  });

  const [config, setConfig] = useState({
    resolution: "1080p",
    bitrate: "12 Mbps",
    fps: "60",
    audioSource: "Game + Mic"
  });

  const [isSyncing, setIsSyncing] = useState(false);
  const [linkingId, setLinkingId] = useState<string | null>(null);
  const [isRepairing, setIsRepairing] = useState(false);
  const [isOpeningPreview, setIsOpeningPreview] = useState(false);
  const [isObsActive, setIsObsActive] = useState(false);

  const logRef = useRef<HTMLDivElement>(null);

  const getRpc = useCallback(() => {
    return window.__mirrorRpc || (window.Electrobun && window.Electrobun.rpc);
  }, []);

  const toggleObsFeed = async () => {
    const newState = !isObsActive;
    try {
        const rpc = getRpc();
        if (rpc) {
            await rpc.request('toggleObsFeed', { enabled: newState });
            setIsObsActive(newState);
        }
    } catch (e) {
        console.error("OBS Feed toggle failed", e);
    }
  };

  // Poll status from Bun process via RPC every 500ms
  useEffect(() => {
    const pollInterval = setInterval(async () => {
      const rpc = getRpc();
      if (!rpc) return;
      try {
        const data = await rpc.request('getStatusUpdate') as StatusUpdate;
        if (data) {
          setStatus(prev => {
            const wasConnected = prev.isConnected;
            const isConnected = data.isActive;
            
            return {
                ...prev,
                decoder: data.decoder || prev.decoder,
                isConnected,
                bufferSize: data.bufferSize,
                devices: data.devices || prev.devices,
                metrics: isConnected ? (data.metrics || prev.metrics) : { throughput_mbps: 0, pipeline_latency_ms: 0, fps_actual: 0, frames_dropped: 0, buffer_health: 0 },
                driverOk: data.driverOk,
                // Clear logs if we just disconnected
                logs: !isConnected && wasConnected 
                ? [] 
                : (data.newLogs && data.newLogs.length > 0
                    ? [...prev.logs, ...data.newLogs].slice(-MAX_LOGS)
                    : prev.logs)
            };
          });
        }
      } catch (e) {
        // RPC not ready yet, ignore
      }
    }, 500);

    return () => clearInterval(pollInterval);
  }, [getRpc]);

  // Auto-scroll logs to bottom when new entries arrive
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [status.logs]);

  const repairDrivers = async () => {
    if (isRepairing) return;
    setIsRepairing(true);
    try {
        const rpc = getRpc();
        if (rpc) await rpc.request('repairDrivers');
    } catch (e) {
        console.error("Repair drivers failed", e);
    } finally {
        setIsRepairing(false);
    }
  };
  
  const connectDevice = async (idInfo: string) => {
    if (linkingId) return;
    setLinkingId(idInfo);
    try {
        const rpc = getRpc();
        if (rpc) {
            const parts = idInfo.split('|');
            const devType = parts[0];
            const id = parts[parts.length - 1]; 
            const [vid, pid] = id.split(':').map(s => parseInt(s, 16));
            
            if (devType === 'Accessory') {
                // Already in accessory mode, just re-enable auto-connect
                await rpc.request('toggleAutoReconnect', { enabled: true });
            } else {
                await rpc.request('triggerHandshake', { vid, pid });
            }
        }
    } catch (e) {
        console.error("Connection failed", e);
    } finally {
        setLinkingId(null);
    }
  };

  const syncConfig = async () => {
    if (isSyncing) return;
    setIsSyncing(true);
    try {
        const rpc = getRpc();
        if (rpc) {
            await rpc.request('syncConfig', { ...config, command: "start" });
        }
    } catch (e) {
        console.error("Sync config failed", e);
    } finally {
        // Short delay for visual feedback
        setTimeout(() => setIsSyncing(false), 500);
    }
  };

  const stopStream = async () => {
    try {
        const rpc = getRpc();
        if (rpc) {
            await rpc.request('syncConfig', { command: "stop" });
        }
    } catch (e) {
        console.error("Stop config failed", e);
    }
  };

  const disconnectDevice = async () => {
    try {
        const rpc = getRpc();
        if (rpc) {
            await rpc.request('syncConfig', { command: "stop" });
        }
    } catch (e) {
        console.error("Disconnect failed", e);
    }
  };

  const openNativePreview = async () => {
    if (isOpeningPreview) return;
    setIsOpeningPreview(true);
    try {
        const rpc = getRpc();
        if (rpc) await rpc.request('openNativePreview');
    } catch (e) {
        console.error("Preview launch failed", e);
    } finally {
        setIsOpeningPreview(false);
    }
  };

  return (
    <div className="min-h-screen bg-[#050608] text-gray-100 font-sans p-6">
      <div className="max-w-6xl mx-auto space-y-6">
        
        {/* Top Status Bar */}
        <div className={`flex justify-between items-center px-4 py-2 rounded-xl border ${status.driverOk ? 'bg-green-900/10 border-green-500/20 text-green-400' : 'bg-yellow-900/10 border-yellow-500/20 text-yellow-400'}`}>
          <div className="flex items-center gap-2 text-[10px] font-black uppercase tracking-widest">
            <div className={`w-1.5 h-1.5 rounded-full ${status.driverOk ? 'bg-green-400' : 'bg-yellow-400 animate-pulse'}`}></div>
            {status.driverOk ? 'System Engine: Operational' : 'Action Required: Driver Permissions'}
          </div>
          {!status.driverOk && (
            <button 
                onClick={repairDrivers} 
                disabled={isRepairing}
                className={`text-[10px] bg-yellow-500 text-black px-3 py-1 rounded font-black uppercase hover:bg-white transition-all cursor-pointer ${isRepairing ? 'opacity-50 cursor-wait' : ''}`}
            >
                {isRepairing ? 'Repairing...' : 'Fix USB Permissions'}
            </button>
          )}
        </div>

        <header className="flex justify-between items-end border-b border-gray-800 pb-6">
          <div>
            <h1 className="text-4xl font-black tracking-tighter text-transparent bg-clip-text bg-gradient-to-r from-orange-400 to-orange-600 uppercase">Mirror Core Enterprise</h1>
            <p className="text-[10px] text-gray-500 font-mono tracking-[0.3em] mt-1 uppercase italic">Unified AV Synchronization Hub</p>
          </div>
          <div className="flex items-center gap-8">
            <div className="text-right">
                <div className="text-[9px] text-gray-500 font-bold uppercase tracking-widest mb-1">Throughput</div>
                <div className={`text-xl font-black ${status.metrics.throughput_mbps > 0 ? 'text-green-400' : 'text-gray-700'}`}>{status.metrics.throughput_mbps.toFixed(2)} <span className="text-[10px]">Mbps</span></div>
            </div>
            <div className="text-right">
                <div className="text-[9px] text-gray-500 font-bold uppercase tracking-widest mb-1">Sync Rate</div>
                <div className={`text-xl font-black ${status.metrics.fps_actual > 0 ? 'text-blue-400' : 'text-gray-700'}`}>{status.metrics.fps_actual.toFixed(1)} <span className="text-[10px]">FPS</span></div>
            </div>
            {status.isConnected && (
                <div className="flex gap-2">
                    {/* OBS Feed Toggle - only show if OBS was detected */}
                    {startupChecks.obsInstalled && (
                    <button 
                        onClick={toggleObsFeed}
                        className={`text-[10px] font-black uppercase px-4 py-3 rounded-xl border transition-all shadow-xl active:scale-[0.98] cursor-pointer ${isObsActive ? 'bg-blue-600 border-blue-400 text-white shadow-blue-900/20' : 'bg-gray-800 border-gray-700 text-gray-400'}`}
                    >
                        {isObsActive ? 'OBS Feed: Active' : 'Direct to OBS'}
                    </button>
                    )}
                    {/* Preview button - only show if ffplay available */}
                    {startupChecks.ffplayOk && (
                    <button 
                        onClick={openNativePreview} 
                        disabled={isOpeningPreview}
                        className={`bg-orange-600 hover:bg-orange-500 text-white text-[10px] font-black uppercase px-6 py-3 rounded-xl transition-all shadow-xl shadow-orange-900/20 active:scale-[0.98] cursor-pointer ${isOpeningPreview ? 'opacity-50 cursor-wait' : ''}`}
                    >
                        {isOpeningPreview ? 'Launching...' : 'Launch Video Preview'}
                    </button>
                    )}
                </div>
            )}
          </div>
        </header>

        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          <div className="lg:col-span-2 space-y-6">
            
            {/* Stream Configuration */}
            {status.isConnected && (
            <section className="bg-[#0e1015] p-6 rounded-2xl border border-gray-800 shadow-2xl">
                <div className="flex justify-between items-center mb-8 pb-4 border-b border-gray-800/50">
                    <h2 className="text-gray-500 text-[9px] font-black uppercase tracking-[0.2em]">Remote Control Settings</h2>
                    <span className="text-[9px] font-black text-orange-500 bg-orange-500/10 px-2 py-1 rounded uppercase tracking-widest">Master Authority</span>
                </div>
                <div className="grid grid-cols-2 md:grid-cols-4 gap-6 text-white">
                    <div className="space-y-2">
                        <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Resolution</label>
                        <select value={config.resolution} onChange={(e) => setConfig({...config, resolution: e.target.value})} className="w-full bg-[#1a1d24] border border-gray-700 rounded-lg p-2.5 text-xs font-bold text-gray-200 cursor-pointer">
                            <option value="720p">720p HD</option>
                            <option value="1080p">1080p FHD</option>
                            <option value="2K">2K QHD</option>
                            <option value="4K">4K UHD</option>
                        </select>
                    </div>
                    <div className="space-y-2">
                        <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Target Bitrate</label>
                        <select value={config.bitrate} onChange={(e) => setConfig({...config, bitrate: e.target.value})} className="w-full bg-[#1a1d24] border border-gray-700 rounded-lg p-2.5 text-xs font-bold text-gray-200 cursor-pointer">
                            <option value="8 Mbps">8 Mbps</option>
                            <option value="12 Mbps">12 Mbps</option>
                            <option value="20 Mbps">20 Mbps</option>
                            <option value="50 Mbps">50 Mbps</option>
                        </select>
                    </div>
                    <div className="space-y-2">
                        <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Framerate</label>
                        <select value={config.fps} onChange={(e) => setConfig({...config, fps: e.target.value})} className="w-full bg-[#1a1d24] border border-gray-700 rounded-lg p-2.5 text-xs font-bold text-gray-200 cursor-pointer">
                            <option value="30">30 FPS</option>
                            <option value="60">60 FPS</option>
                            <option value="90">90 FPS</option>
                            <option value="120">120 FPS</option>
                        </select>
                    </div>
                    <div className="space-y-2">
                        <label className="text-[9px] font-bold text-gray-500 uppercase tracking-widest">Audio Routing</label>
                        <select value={config.audioSource} onChange={(e) => setConfig({...config, audioSource: e.target.value})} className="w-full bg-[#1a1d24] border border-gray-700 rounded-lg p-2.5 text-xs font-bold text-orange-400 cursor-pointer">
                            <option value="Game System">Game System</option>
                            <option value="Microphone">Microphone</option>
                            <option value="Game + Mic">Game + Mic</option>
                            <option value="Mute All">Mute All</option>
                        </select>
                    </div>
                </div>
                <div className="flex gap-4 mt-8">
                    <button 
                        onClick={syncConfig}
                        disabled={isSyncing}
                        className={`flex-1 py-3 bg-green-500/10 hover:bg-green-500/20 text-green-400 text-[10px] font-black uppercase rounded-xl border border-green-500/20 transition-all active:scale-[0.98] cursor-pointer ${isSyncing ? 'opacity-50 cursor-wait' : ''}`}
                    >
                        {isSyncing ? 'Starting Pipeline...' : 'Start Capture & Sync Parameters'}
                    </button>
                    <button 
                        onClick={stopStream}
                        className="flex-1 py-3 bg-red-500/10 hover:bg-red-500/20 text-red-500 text-[10px] font-black uppercase rounded-xl border border-red-500/20 transition-all active:scale-[0.98] cursor-pointer"
                    >
                        Stop Mobile Capture
                    </button>
                </div>
            </section>
            )}

            {/* Target Discovery */}
            <section className="bg-[#0e1015] p-6 rounded-2xl border border-gray-800 shadow-2xl">
                <h2 className="text-gray-500 text-[9px] font-black uppercase mb-6 tracking-[0.2em]">Live USB Discovery</h2>
                <div className="space-y-3">
                    {status.devices.length === 0 ? (
                        <div className="flex items-center gap-3 text-sm text-gray-600 italic py-4 animate-pulse">Scanning high-speed USB bus...</div>
                    ) : (
                        status.devices.map((dev) => {
                            const [devType, name, id] = dev.split('|');
                            const isAccessory = devType === 'Accessory';
                            const isStreaming = isAccessory && status.isConnected;
                            const isLinking = linkingId === dev;
                            return (
                                <div key={id} className={`flex justify-between items-center bg-black/40 p-4 rounded-xl border transition-all ${isStreaming ? 'border-green-500/30' : 'border-gray-800/50 hover:border-orange-500/30'}`}>
                                    <div className="flex items-center gap-4">
                                        <div className={`w-2 h-2 rounded-full ${isStreaming ? 'bg-green-400 shadow-[0_0_10px_#4ade80]' : 'bg-blue-400'}`}></div>
                                        <div>
                                            <div className="text-sm font-black text-white leading-none mb-1">{name}</div>
                                            <div className="text-[9px] text-gray-500 font-bold uppercase font-mono">{id} // {devType}</div>
                                        </div>
                                    </div>
                                    <button 
                                        onClick={() => isStreaming ? disconnectDevice() : connectDevice(dev)} 
                                        disabled={isLinking}
                                        className={`text-[10px] font-black uppercase px-6 py-2 rounded-lg border transition-all cursor-pointer ${isStreaming ? 'bg-red-500/10 text-red-400 border-red-500/20 hover:bg-red-500 hover:text-white' : 'bg-orange-500/10 text-orange-500 border-orange-500/20 hover:bg-orange-500 hover:text-black'} ${isLinking ? 'opacity-50 cursor-wait' : ''}`}
                                    >
                                        {isStreaming ? 'Disconnect' : isLinking ? 'Linking...' : 'Initiate'}
                                    </button>
                                </div>
                            );
                        })
                    )}
                </div>
            </section>
          </div>

          {/* Real-time Logs */}
          <section className="bg-[#0e1015] rounded-2xl border border-gray-800 shadow-2xl flex flex-col h-[650px]">
            <div className="p-4 border-b border-gray-800 flex justify-between items-center bg-black/20">
                <h2 className="text-gray-500 text-[9px] font-black uppercase tracking-[0.2em]">Diagnostic Stream</h2>
                <div className="flex items-center gap-2">
                    <span className="text-[8px] font-black text-gray-600 uppercase">Live</span>
                    <div className="w-1.5 h-1.5 rounded-full bg-orange-500 animate-pulse"></div>
                </div>
            </div>
            <div ref={logRef} className="flex-1 overflow-y-auto p-4 space-y-1 custom-scrollbar scroll-smooth">
                {status.logs.length === 0 ? (
                    <div className="text-gray-700 text-[10px] italic">Awaiting engine events...</div>
                ) : (
                    status.logs.map((log, i) => <LogItem key={`${log.timestamp}-${i}`} log={log} />)
                )}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
//  APP ROOT — Manages Loader → Dashboard transition
// ═══════════════════════════════════════════════════════════

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
