import type { StartupChecks } from '@/types';
import { DeviceList } from './DeviceList';
import { LogPanel } from './LogPanel';
import { MetricsHeader } from './MetricsHeader';
import { SinkControls } from './SinkControls';
import { StatusBar } from './StatusBar';
import { StreamSettings } from './StreamSettings';
import { useStatusPolling } from './useStatusPolling';

interface DashboardProps {
  startupChecks: StartupChecks;
}

/**
 * Layout for the main screen.
 *
 * Deliberately holds no state and makes no RPC calls of its own: polling lives
 * in {@link useStatusPolling} and each panel owns whatever local state it
 * needs. When this file was one 482-line component, adding a control meant
 * touching the same `useState` block and the same JSX tree as every other
 * feature.
 */
export function Dashboard({ startupChecks }: DashboardProps) {
  const status = useStatusPolling(startupChecks.driverOk);

  return (
    <div className="min-h-screen bg-[#050608] p-6 font-sans text-gray-100">
      <div className="mx-auto max-w-6xl space-y-6">
        <StatusBar driverOk={status.driverOk} />

        <MetricsHeader metrics={status.metrics} isConnected={status.isConnected}>
          <SinkControls
            startupChecks={startupChecks}
            obsRunning={status.obsRunning}
            isConnected={status.isConnected}
            playerState={status.playerState}
          />
        </MetricsHeader>

        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <div className="space-y-6 lg:col-span-2">
            {status.isConnected && <StreamSettings />}
            <DeviceList devices={status.devices} isConnected={status.isConnected} />
          </div>

          <LogPanel logs={status.logs} />
        </div>
      </div>
    </div>
  );
}
