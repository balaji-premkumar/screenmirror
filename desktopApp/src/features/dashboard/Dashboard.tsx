import { useState, useEffect, useCallback, useRef } from "react";
import { StartupChecks, StatusUpdate, LogEntry, Metrics } from "@/types";
import { LogItem } from "./LogItem";

interface DashboardProps {
  startupChecks: StartupChecks;
}

export const Dashboard: React.FC<DashboardProps> = ({ startupChecks }) => {
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
  
  const missedPolls = useRef(0);
  const logRef = useRef<HTMLDivElement>(null);

  const getRpc = useCallback(() => {
    // @ts-ignore
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

  useEffect(() => {
    let cancelled = false;
    const pollInterval = setInterval(async () => {
      const rpc = getRpc();
      if (!rpc || cancelled) return;
      try {
        const data = await rpc.request('getStatusUpdate') as StatusUpdate;
        if (data && !cancelled) {
          setStatus(prev => {
            let nextIsConnected = data.isActive;
            if (!data.isActive) {
                missedPolls.current++;
                if (missedPolls.current < 3 && prev.isConnected) {
                    nextIsConnected = true; 
                }
            } else {
                missedPolls.current = 0;
            }

            const isConnectedChanged = prev.isConnected !== nextIsConnected;
            const driverOkChanged = prev.driverOk !== data.driverOk;
            const newDevices = data.devices || [];
            const devicesChanged = JSON.stringify(prev.devices) !== JSON.stringify(newDevices);
            const newMetrics = nextIsConnected ? (data.metrics || prev.metrics) : { throughput_mbps: 0, pipeline_latency_ms: 0, fps_actual: 0, frames_dropped: 0, buffer_health: 0 };
            const hasNewLogs = data.newLogs && data.newLogs.length > 0;

            if (!isConnectedChanged && !driverOkChanged && !devicesChanged && prev.bufferSize === data.bufferSize && !hasNewLogs) {
                return prev;
            }

            return {
              ...prev,
              decoder: data.decoder || prev.decoder,
              isConnected: nextIsConnected,
              bufferSize: data.bufferSize,
              devices: devicesChanged ? newDevices : prev.devices,
              metrics: newMetrics,
              driverOk: data.driverOk,
              logs: hasNewLogs ? [...prev.logs, ...data.newLogs].slice(-300) : (isConnectedChanged && !nextIsConnected ? [] : prev.logs)
            };
          });
        }
      } catch (e) {}
    }, 500);

    return () => {
        cancelled = true;
        clearInterval(pollInterval);
    };
  }, [getRpc]);

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
      console.error("Repair failed", e);
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
      console.error("Sync failed", e);
    } finally {
      setTimeout(() => setIsSyncing(false), 500);
    }
  };

  const disconnectDevice = async () => {
    try {
      const rpc = getRpc();
      if (rpc) await rpc.request('disconnectDevice');
    } catch (e) {}
  };

  const openNativePreview = async () => {
    if (isOpeningPreview) return;
    setIsOpeningPreview(true);
    try {
      const rpc = getRpc();
      if (rpc) await rpc.request('openNativePreview');
    } catch (e) {} finally {
      setIsOpeningPreview(false);
    }
  };

  return (
    <div className="flex flex-col h-screen bg-[#0A0A0B] text-[#E0E2E8]">
      
      {/* Top Navigation / Status */}
      <nav className="h-16 border-b border-white/10 flex items-center justify-between px-8 shrink-0">
        <div className="flex items-center gap-4">
          <div className="w-8 h-8 bg-[#00F0FF] rounded-md flex items-center justify-center">
            <div className="w-4 h-4 bg-black rotate-45"></div>
          </div>
          <h1 className="text-xl font-bold tracking-tight uppercase">ScreenMirror <span className="text-[#00F0FF]">Pro</span></h1>
        </div>

        <div className="flex items-center gap-6">
          <div className="flex items-center gap-2 px-3 py-1 bg-white/5 rounded border border-white/10">
            <div className={`w-2 h-2 rounded-full ${status.isConnected ? 'bg-[#00F0FF] live-pulse' : 'bg-[#8E9196]'}`}></div>
            <span className="text-[10px] font-bold uppercase tracking-widest">
              {status.isConnected ? 'Stream Active' : 'Disconnected'}
            </span>
          </div>

          {!status.driverOk && (
            <button onClick={repairDrivers} className="precision-button-primary py-1 px-4 text-[10px]">
              {isRepairing ? 'Repairing...' : 'Fix Drivers'}
            </button>
          )}

          <div className="flex gap-2">
            {startupChecks.obsInstalled && (
              <button 
                onClick={toggleObsFeed}
                className={`text-[10px] font-bold uppercase px-4 py-2 rounded border transition-all ${isObsActive ? 'bg-[#00F0FF] text-black border-[#00F0FF]' : 'border-white/20 text-[#8E9196] hover:border-[#00F0FF] hover:text-[#00F0FF]'}`}
              >
                OBS Feed: {isObsActive ? 'ON' : 'OFF'}
              </button>
            )}
            {status.isConnected && startupChecks.ffplayOk && (
              <button onClick={openNativePreview} className="precision-button-secondary py-2 px-4 text-[10px]">
                {isOpeningPreview ? 'Loading...' : 'Native Preview'}
              </button>
            )}
          </div>
        </div>
      </nav>

      {/* Main Content Area */}
      <main className="flex-1 flex overflow-hidden p-6 gap-6">
        
        {/* Left Column: Metrics & Config */}
        <div className="flex-[2] flex flex-col gap-6">
          
          {/* Quick Metrics Grid */}
          <div className="grid grid-cols-4 gap-4">
            {[
              { label: 'Throughput', val: (status.metrics.throughput_mbps ?? 0).toFixed(1), unit: 'Mbps', color: '#00F0FF' },
              { label: 'Latency', val: (status.metrics.pipeline_latency_ms ?? 0).toFixed(0), unit: 'ms', color: (status.metrics.pipeline_latency_ms ?? 0) < 30 ? '#4ade80' : '#f87171' },
              { label: 'Framerate', val: (status.metrics.fps_actual ?? 0).toFixed(0), unit: 'FPS', color: '#00F0FF' },
              { label: 'Drops', val: status.metrics.frames_dropped ?? 0, unit: 'Frames', color: '#f87171' },
            ].map((m, i) => (
              <div key={i} className="precision-card p-4 flex flex-col justify-between">
                <span className="text-[9px] font-bold text-[#8E9196] uppercase tracking-widest">{m.label}</span>
                <div className="flex items-baseline gap-1 mt-2">
                  <span className="text-2xl font-tech font-bold" style={{ color: m.color }}>{m.val}</span>
                  <span className="text-[10px] text-[#8E9196] font-bold">{m.unit}</span>
                </div>
              </div>
            ))}
          </div>

          {/* Configuration & Controls */}
          <div className="flex-1 precision-card p-6 flex flex-col">
            <div className="flex justify-between items-center mb-6">
              <h2 className="text-xs font-bold uppercase tracking-widest">Stream Parameters</h2>
              <span className="text-[10px] text-[#00F0FF] bg-[#00F0FF]/10 px-2 py-0.5 rounded border border-[#00F0FF]/20 font-bold">KINETIC MODE</span>
            </div>

            <div className="grid grid-cols-2 gap-x-8 gap-y-6 flex-1">
              {[
                { label: 'Resolution', key: 'resolution', options: ['720p', '1080p', '2K', '4K'] },
                { label: 'Bitrate', key: 'bitrate', options: ['8 Mbps', '12 Mbps', '25 Mbps', '50 Mbps'] },
                { label: 'Target FPS', key: 'fps', options: ['30', '60', '90', '120'] },
                { label: 'Audio Source', key: 'audioSource', options: ['System', 'Microphone', 'Both', 'Muted'] },
              ].map((field) => (
                <div key={field.key} className="flex flex-col gap-2">
                  <label className="text-[9px] font-bold text-[#8E9196] uppercase tracking-widest">{field.label}</label>
                  <div className="relative group">
                    <select 
                      value={(config as any)[field.key]} 
                      onChange={(e) => setConfig({...config, [field.key]: e.target.value})}
                      className="w-full bg-black/40 border border-white/10 rounded p-2 text-xs font-bold focus:border-[#00F0FF] focus:outline-none appearance-none transition-all cursor-pointer"
                    >
                      {field.options.map(opt => <option key={opt} value={opt}>{opt}</option>)}
                    </select>
                    <div className="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none text-[#8E9196] group-hover:text-[#00F0FF]">▼</div>
                  </div>
                </div>
              ))}
            </div>

            <div className="flex gap-4 mt-8">
              <button onClick={syncConfig} disabled={!status.isConnected || isSyncing} className="precision-button-primary flex-1">
                {isSyncing ? 'Syncing...' : 'Sync & Start'}
              </button>
              <button onClick={disconnectDevice} disabled={!status.isConnected} className="precision-button-secondary border-red-500/50 text-red-500 hover:bg-red-500/10 flex-1">
                Disconnect
              </button>
            </div>
          </div>

          {/* Device Discovery */}
          <div className="precision-card p-6 overflow-hidden flex flex-col min-h-[180px]">
            <h2 className="text-xs font-bold uppercase tracking-widest mb-4">USB Node Map</h2>
            <div className="space-y-2 overflow-y-auto pr-2 custom-scrollbar">
              {status.devices.length === 0 ? (
                <div className="text-xs text-[#8E9196] italic py-4 border border-dashed border-white/5 rounded text-center uppercase tracking-tighter">Scanning High-Speed Interconnects...</div>
              ) : (
                status.devices.map((dev) => {
                  const [devType, name, id] = dev.split('|');
                  const isStreaming = devType === 'Accessory' && status.isConnected;
                  return (
                    <div key={id} className={`flex items-center justify-between p-3 bg-black/20 rounded border ${isStreaming ? 'border-[#00F0FF]/30' : 'border-white/5'}`}>
                      <div className="flex items-center gap-3">
                        <div className={`w-1.5 h-1.5 rounded-full ${isStreaming ? 'bg-[#00F0FF]' : 'bg-[#8E9196]'}`}></div>
                        <div>
                          <p className="text-xs font-bold">{name}</p>
                          <p className="text-[9px] font-tech text-[#8E9196]">{id} // {devType}</p>
                        </div>
                      </div>
                      <button 
                        onClick={() => isStreaming ? disconnectDevice() : connectDevice(dev)}
                        className={`text-[9px] font-bold uppercase px-3 py-1 rounded border transition-all ${isStreaming ? 'border-red-500/50 text-red-400 hover:bg-red-500 hover:text-black' : 'border-[#00F0FF]/50 text-[#00F0FF] hover:bg-[#00F0FF] hover:text-black'}`}
                      >
                        {isStreaming ? 'Terminate' : 'Initialize'}
                      </button>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>

        {/* Right Column: Diagnostics */}
        <div className="flex-1 precision-card flex flex-col overflow-hidden">
          <div className="p-4 border-b border-white/10 bg-black/20 flex justify-between items-center">
            <span className="text-xs font-bold uppercase tracking-widest">Diagnostic Stream</span>
            <div className="flex items-center gap-1.5">
              <span className="text-[8px] font-bold text-[#8E9196]">LIVE</span>
              <div className="w-1 h-1 rounded-full bg-[#00F0FF] animate-ping"></div>
            </div>
          </div>
          <div ref={logRef} className="flex-1 overflow-y-auto p-4 space-y-1 font-tech custom-scrollbar">
            {status.logs.length === 0 ? (
              <div className="text-[10px] text-[#8E9196] uppercase italic">Awaiting telemetry...</div>
            ) : (
              status.logs.map((log, i) => <LogItem key={i} log={log} />)
            )}
          </div>
        </div>

      </main>

      {/* Footer Info */}
      <footer className="h-8 border-t border-white/10 px-8 flex items-center justify-between shrink-0 bg-black/40">
        <div className="text-[9px] text-[#8E9196] font-bold uppercase tracking-tighter">
          Engine: <span className="text-[#E0E2E8]">RUST_BACKEND_2.4.0</span> // Decoder: <span className="text-[#00F0FF]">{status.decoder}</span>
        </div>
        <div className="text-[9px] text-[#8E9196] font-tech font-bold uppercase">
          Buffer Usage: <span className={status.bufferSize > 15 ? 'text-red-400' : 'text-[#00F0FF]'}>{status.bufferSize}/20 Frames</span>
        </div>
      </footer>
    </div>
  );
};
