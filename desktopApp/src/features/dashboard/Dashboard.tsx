import { useEffect, useState } from 'react';
import LogoIcon from '@/components/ui/LogoIcon';
import { tryCall } from '@/services/rpc';
import { useT } from '@/i18n';
import { PlayerState, parseDevice, type StartupChecks } from '@/types';
import { HealthPanel } from './HealthPanel';
import { LinkStage } from './LinkStage';
import { LogPanel } from './LogPanel';
import { OtherDevices } from './DeviceList';
import { QualityPanel } from './QualityPanel';
import { SetupBanner } from './SetupBanner';
import { SinkControls } from './SinkControls';
import { assessStream, deriveStage, primaryDevice } from './stage';
import { useSession } from './useSession';
import { useStatusPolling } from './useStatusPolling';

interface DashboardProps {
  startupChecks: StartupChecks;
}

/**
 * Layout for the main screen.
 *
 * Polling lives in {@link useStatusPolling}, the person's actions in
 * {@link useSession}, and the wording in `stage.ts`; this file only arranges
 * them. The order on screen follows the order of the task: see what is
 * happening, choose quality, check the stream is healthy.
 */
export function Dashboard({ startupChecks: initialChecks }: DashboardProps) {
  const t = useT();
  const [startupChecks, setStartupChecks] = useState(initialChecks);

  // The loader may have given up on the backend (slow startup) or OBS may have
  // been installed since. Ask again once, so the sink controls are not hidden
  // for the life of the window by a stale "missing".
  useEffect(() => {
    if (initialChecks.ffplayOk && initialChecks.obsInstalled && initialChecks.obsPluginInstalled) return;
    void tryCall('getStartupChecks', { fallback: initialChecks }).then(setStartupChecks);
  }, [initialChecks]);
  const status = useStatusPolling(startupChecks.driverOk);
  const fps = status.metrics.fps_actual;
  const session = useSession(status.isConnected, fps);

  const stage = deriveStage({
    devices: status.devices,
    isConnected: status.isConnected,
    fps,
    startRequested: session.startRequested,
  });
  const device = primaryDevice(status.devices);
  const others = status.devices.map(parseDevice).filter((d) => d.raw !== device?.raw);

  const isMirroring = stage === 'mirroring';
  const health = isMirroring ? assessStream(status.metrics, Number(session.config.fps)) : null;

  return (
    <div className="mx-auto flex min-h-screen max-w-5xl flex-col gap-8 px-5 py-6 sm:px-8">
      <header className="flex items-center gap-2.5">
        <LogoIcon />
        <span className="text-lg font-semibold tracking-tight">{t('app.title')}</span>
      </header>

      {!status.driverOk && <SetupBanner />}

      <LinkStage
        stage={stage}
        device={device}
        fps={fps}
        verdict={
          health
            ? {
                level: health.level,
                text: t(health.verdictKey),
                hint: health.hintKey ? t(health.hintKey) : undefined,
              }
            : undefined
        }
        session={session}
        sinks={
          <SinkControls
            startupChecks={startupChecks}
            obsRunning={status.obsRunning}
            playerState={status.playerState ?? PlayerState.Stopped}
          />
        }
      />

      <div className="grid gap-10 lg:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
        <QualityPanel session={session} isMirroring={isMirroring} />
        <HealthPanel metrics={status.metrics} isMirroring={isMirroring} />
      </div>

      <OtherDevices devices={others} session={session} />

      <div className="mt-auto">
        <LogPanel logs={status.logs} />
      </div>
    </div>
  );
}
