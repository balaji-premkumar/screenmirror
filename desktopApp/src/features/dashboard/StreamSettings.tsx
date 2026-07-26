import { useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';
import type { StreamConfig } from '@/types';

/**
 * Choices the phone understands.
 *
 * The values are sent verbatim and parsed on the Android side, so they are not
 * translated — only the labels around them are. Changing a value here means
 * changing `MirrorForegroundService.kt` too.
 */
const RESOLUTIONS = ['720p', '1080p', '2K', '4K'] as const;
const BITRATES = ['8 Mbps', '12 Mbps', '20 Mbps', '50 Mbps'] as const;
const FRAMERATES = ['30', '60', '90', '120'] as const;

const AUDIO_SOURCES = [
  { value: 'Game System', labelKey: 'settings.audio.system' },
  { value: 'Microphone', labelKey: 'settings.audio.mic' },
  { value: 'Game + Mic', labelKey: 'settings.audio.both' },
  { value: 'Mute All', labelKey: 'settings.audio.mute' },
] as const;

const DEFAULT_CONFIG: StreamConfig = {
  resolution: '1080p',
  bitrate: '12 Mbps',
  fps: '60',
  audioSource: 'Game + Mic',
};

const SELECT_CLASS =
  'w-full cursor-pointer rounded-lg border border-gray-700 bg-[#1a1d24] p-2.5 text-xs font-bold text-gray-200';

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <label className="text-[9px] font-bold uppercase tracking-widest text-gray-500">
        {label}
      </label>
      {children}
    </div>
  );
}

/** Capture settings pushed to the phone, and the start/stop/disconnect actions. */
export function StreamSettings() {
  const t = useT();
  const [config, setConfig] = useState<StreamConfig>(DEFAULT_CONFIG);
  const [isStarting, setIsStarting] = useState(false);

  const update = <K extends keyof StreamConfig>(key: K, value: StreamConfig[K]) =>
    setConfig((prev) => ({ ...prev, [key]: value }));

  const start = async () => {
    if (isStarting) return;
    setIsStarting(true);
    try {
      await call('syncConfig', { ...config, command: 'start' });
    } catch (error) {
      console.error('Starting capture failed', error);
    } finally {
      // Held briefly so the state change is visible; the phone still has to
      // show its own capture-consent dialog before anything streams.
      setTimeout(() => setIsStarting(false), 500);
    }
  };

  const stop = () =>
    call('syncConfig', { command: 'stop' }).catch((error) =>
      console.error('Stopping capture failed', error),
    );

  const disconnect = () =>
    call('disconnectDevice').catch((error) =>
      console.error('Disconnecting failed', error),
    );

  return (
    <section className="rounded-2xl border border-gray-800 bg-[#0e1015] p-6 shadow-2xl">
      <div className="mb-8 flex items-center justify-between border-b border-gray-800/50 pb-4">
        <h2 className="text-[9px] font-black uppercase tracking-[0.2em] text-gray-500">
          {t('settings.title')}
        </h2>
        <span className="rounded bg-orange-500/10 px-2 py-1 text-[9px] font-black uppercase tracking-widest text-orange-500">
          {t('settings.badge')}
        </span>
      </div>

      <div className="grid grid-cols-2 gap-6 text-white md:grid-cols-4">
        <Field label={t('settings.resolution')}>
          <select
            value={config.resolution}
            onChange={(e) => update('resolution', e.target.value)}
            className={SELECT_CLASS}
          >
            {RESOLUTIONS.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </Field>

        <Field label={t('settings.bitrate')}>
          <select
            value={config.bitrate}
            onChange={(e) => update('bitrate', e.target.value)}
            className={SELECT_CLASS}
          >
            {BITRATES.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </Field>

        <Field label={t('settings.framerate')}>
          <select
            value={config.fps}
            onChange={(e) => update('fps', e.target.value)}
            className={SELECT_CLASS}
          >
            {FRAMERATES.map((value) => (
              <option key={value} value={value}>
                {value} {t('metrics.framerate.unit')}
              </option>
            ))}
          </select>
        </Field>

        <Field label={t('settings.audio')}>
          <select
            value={config.audioSource}
            onChange={(e) => update('audioSource', e.target.value)}
            className={`${SELECT_CLASS} text-orange-400`}
          >
            {AUDIO_SOURCES.map(({ value, labelKey }) => (
              <option key={value} value={value}>
                {t(labelKey)}
              </option>
            ))}
          </select>
        </Field>
      </div>

      <div className="mt-8 flex gap-4">
        <button
          type="button"
          onClick={start}
          disabled={isStarting}
          className={`flex-1 cursor-pointer rounded-xl border border-green-500/20 bg-green-500/10 py-3 text-[10px] font-black uppercase text-green-400 transition-all hover:bg-green-500/20 active:scale-[0.98] ${
            isStarting ? 'cursor-wait opacity-50' : ''
          }`}
        >
          {isStarting ? t('settings.starting') : t('settings.start')}
        </button>
        <button
          type="button"
          onClick={stop}
          className="flex-1 cursor-pointer rounded-xl border border-yellow-500/20 bg-yellow-500/10 py-3 text-[10px] font-black uppercase text-yellow-500 transition-all hover:bg-yellow-500/20 active:scale-[0.98]"
        >
          {t('settings.stop')}
        </button>
        <button
          type="button"
          onClick={disconnect}
          className="flex-1 cursor-pointer rounded-xl border border-red-500/20 bg-red-500/10 py-3 text-[10px] font-black uppercase text-red-500 transition-all hover:bg-red-500/20 active:scale-[0.98]"
        >
          {t('settings.disconnect')}
        </button>
      </div>
    </section>
  );
}
