import { useEffect, useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';
import { PlayerState, type StartupChecks } from '@/types';

interface SinkControlsProps {
  startupChecks: StartupChecks;
  obsRunning: boolean;
  isConnected: boolean;
  playerState: PlayerState;
}

/**
 * The two buttons that decide where the stream goes.
 *
 * These are the *only* two sinks. The app itself never opens an audio device:
 * sound reaches the machine either through the child ffplay process or through
 * the OBS shared-memory feed, and only after the user asks for it here.
 */
export function SinkControls({
  startupChecks,
  obsRunning,
  isConnected,
  playerState,
}: SinkControlsProps) {
  const t = useT();
  const [isObsActive, setIsObsActive] = useState(false);
  const [isTogglingPlayer, setIsTogglingPlayer] = useState(false);

  // Sending frames is only meaningful when the plugin is installed *and* OBS
  // is running to read the shared memory. Otherwise it is an 8 MB memcpy per
  // frame into a buffer nobody maps.
  const obsReady =
    startupChecks.obsInstalled && startupChecks.obsPluginInstalled && obsRunning;

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

  return (
    <>
      {startupChecks.obsInstalled && (
        <button
          type="button"
          onClick={toggleObs}
          disabled={!obsReady}
          title={obsTooltip}
          className={`flex items-center gap-2 rounded-xl border px-4 py-3 text-[10px] font-black uppercase shadow-xl transition-all active:scale-[0.98] ${
            !obsReady
              ? 'cursor-not-allowed border-gray-800 bg-gray-900 text-gray-600'
              : isObsActive
                ? 'cursor-pointer border-blue-400 bg-blue-600 text-white shadow-blue-900/20'
                : 'cursor-pointer border-gray-700 bg-gray-800 text-gray-400'
          }`}
        >
          <span
            className={`h-1.5 w-1.5 rounded-full ${
              isObsActive ? 'animate-pulse bg-white' : obsReady ? 'bg-blue-400' : 'bg-gray-700'
            }`}
          />
          {obsLabel}
        </button>
      )}

      {isConnected && startupChecks.ffplayOk && (
        <button
          type="button"
          onClick={togglePlayer}
          disabled={isTogglingPlayer || playerState === PlayerState.Starting}
          title={
            playerState === PlayerState.Stopped
              ? t('player.tooltip.start')
              : t('player.tooltip.stop')
          }
          className={`flex items-center gap-2 rounded-xl px-6 py-3 text-[10px] font-black uppercase text-white shadow-xl transition-all active:scale-[0.98] ${
            playerState === PlayerState.Playing
              ? 'cursor-pointer bg-red-600 shadow-red-900/20 hover:bg-red-500'
              : 'cursor-pointer bg-orange-600 shadow-orange-900/20 hover:bg-orange-500'
          } ${
            isTogglingPlayer || playerState === PlayerState.Starting
              ? 'cursor-wait opacity-60'
              : ''
          }`}
        >
          {playerState === PlayerState.Playing ? '■' : '▶'}
          {playerLabel}
        </button>
      )}
    </>
  );
}
