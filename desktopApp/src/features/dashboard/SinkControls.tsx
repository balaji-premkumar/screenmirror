import { useEffect, useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';
import { PlayerState, type StartupChecks } from '@/types';

interface SinkControlsProps {
  startupChecks: StartupChecks;
  obsRunning: boolean;
  playerState: PlayerState;
}

const BUTTON =
  'inline-flex items-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-50';

/**
 * The two places the picture can go besides the phone's own screen.
 *
 * These are the *only* two sinks. The app itself never opens an audio device:
 * sound reaches the machine either through the child ffplay process or through
 * the OBS shared-memory feed, and only after the user asks for it here.
 */
export function SinkControls({ startupChecks, obsRunning, playerState }: SinkControlsProps) {
  const t = useT();
  const [isObsActive, setIsObsActive] = useState(false);
  const [isTogglingPlayer, setIsTogglingPlayer] = useState(false);

  // Sending frames is only meaningful when the plugin is installed *and* OBS
  // is running to read the shared memory. Otherwise it is an 8 MB memcpy per
  // frame into a buffer nobody maps.
  const obsReady = startupChecks.obsInstalled && startupChecks.obsPluginInstalled && obsRunning;

  // If OBS quits while the feed is on, stop writing.
  useEffect(() => {
    if (!isObsActive || obsReady) return;
    void call('toggleObsFeed', { enabled: false }).catch(() => {});
    setIsObsActive(false);
  }, [isObsActive, obsReady]);

  const toggleObs = async () => {
    if (!obsReady) return;
    const enabled = !isObsActive;
    try {
      await call('toggleObsFeed', { enabled });
      setIsObsActive(enabled);
    } catch (error) {
      console.error('Toggling the OBS feed failed', error);
    }
  };

  const togglePlayer = async () => {
    if (isTogglingPlayer) return;
    setIsTogglingPlayer(true);
    try {
      await call(playerState === PlayerState.Stopped ? 'startPlayer' : 'stopPlayer');
    } catch (error) {
      console.error('Toggling playback failed', error);
    } finally {
      setIsTogglingPlayer(false);
    }
  };

  const obsLabel = isObsActive
    ? t('obs.live')
    : !startupChecks.obsPluginInstalled
      ? t('obs.pluginMissing')
      : !obsRunning
        ? t('obs.notRunning')
        : t('obs.send');

  const obsTooltip = !startupChecks.obsPluginInstalled
    ? t('obs.tooltip.installPlugin')
    : !obsRunning
      ? t('obs.tooltip.startObs')
      : isObsActive
        ? t('obs.tooltip.stop')
        : t('obs.tooltip.start');

  const playerLabel =
    playerState === PlayerState.Starting
      ? t('player.buffering')
      : playerState === PlayerState.Playing
        ? t('player.stop')
        : t('player.play');

  const playing = playerState === PlayerState.Playing;

  return (
    <>
      {startupChecks.ffplayOk && (
        <button
          type="button"
          onClick={togglePlayer}
          disabled={isTogglingPlayer || playerState === PlayerState.Starting}
          aria-pressed={playing}
          title={playerState === PlayerState.Stopped ? t('player.tooltip.start') : t('player.tooltip.stop')}
          className={`${BUTTON} ${
            playing
              ? 'border-signal bg-signal/10 text-signal'
              : 'border-line bg-panel text-ink hover:border-ink/40'
          }`}
        >
          {playerLabel}
        </button>
      )}

      {startupChecks.obsInstalled && (
        <button
          type="button"
          onClick={toggleObs}
          disabled={!obsReady}
          aria-pressed={isObsActive}
          title={obsTooltip}
          className={`${BUTTON} ${
            isObsActive
              ? 'border-signal bg-signal/10 text-signal'
              : 'border-line bg-panel text-ink hover:border-ink/40'
          }`}
        >
          {isObsActive && <span className="h-2 w-2 rounded-full bg-signal motion-safe:animate-pulse" />}
          {obsLabel}
        </button>
      )}
    </>
  );
}
