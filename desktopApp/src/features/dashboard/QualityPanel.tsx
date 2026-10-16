import { useT } from '@/i18n';
import { Segmented } from './Segmented';
import { PRESETS } from './stage';
import type { Session } from './useSession';

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

interface QualityPanelProps {
  session: Session;
  isMirroring: boolean;
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-2 sm:grid-cols-[8.5rem_1fr] sm:items-center">
      <span className="text-sm text-muted">{label}</span>
      {children}
    </div>
  );
}

/** Picture quality: four ready-made choices, and every dial behind them. */
export function QualityPanel({ session, isMirroring }: QualityPanelProps) {
  const t = useT();
  const { config, activePreset } = session;

  return (
    <section aria-labelledby="quality-title">
      <h2 id="quality-title" className="text-xl font-semibold tracking-tight">
        {t('quality.title')}
      </h2>
      <p className="mt-1 text-muted">
        {isMirroring ? t('quality.appliesNext') : t('quality.intro')}
      </p>

      <div role="radiogroup" aria-label={t('quality.title')} className="mt-5 grid grid-cols-2 gap-3">
        {PRESETS.map((preset) => {
          const selected = preset.id === activePreset;
          return (
            <button
              key={preset.id}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => session.applyPreset(preset.id)}
              className={`flex flex-col items-start justify-start rounded-2xl border p-4 text-left transition-colors ${
                selected ? 'border-signal bg-signal/10' : 'border-line bg-panel hover:border-ink/40'
              }`}
            >
              <span className={`block text-base font-semibold ${selected ? 'text-signal' : ''}`}>
                {t(`quality.preset.${preset.id}`)}
              </span>
              <span className="num mt-1 block text-sm text-muted">
                {preset.resolution}, {preset.fps} {t('metrics.framerate.unit')}
              </span>
              <span className="mt-2 block text-sm leading-snug text-muted">
                {t(`quality.preset.${preset.id}.hint`)}
              </span>
            </button>
          );
        })}
      </div>

      <details className="group mt-4" open={!activePreset}>
        <summary className="inline-flex items-center gap-1.5 rounded-lg py-1 text-sm font-semibold text-signal">
          <span className="transition-transform group-open:rotate-90" aria-hidden="true">
            ›
          </span>
          {activePreset ? t('quality.fineTune') : t('quality.custom')}
        </summary>

        <div className="mt-4 space-y-4 rounded-2xl border border-line bg-panel p-5">
          <Row label={t('settings.resolution')}>
            <Segmented
              label={t('settings.resolution')}
              value={config.resolution}
              options={RESOLUTIONS.map((v) => ({ value: v, label: v }))}
              onChange={(v) => session.update('resolution', v)}
            />
          </Row>
          <Row label={t('settings.framerate')}>
            <Segmented
              label={t('settings.framerate')}
              value={config.fps}
              options={FRAMERATES.map((v) => ({ value: v, label: `${v} ${t('metrics.framerate.unit')}` }))}
              onChange={(v) => session.update('fps', v)}
            />
          </Row>
          <Row label={t('settings.bitrate')}>
            <Segmented
              label={t('settings.bitrate')}
              value={config.bitrate}
              options={BITRATES.map((v) => ({ value: v, label: v }))}
              onChange={(v) => session.update('bitrate', v)}
            />
          </Row>
          <Row label={t('settings.audio')}>
            <Segmented
              label={t('settings.audio')}
              value={config.audioSource}
              options={AUDIO_SOURCES.map((s) => ({ value: s.value, label: t(s.labelKey) }))}
              onChange={(v) => session.update('audioSource', v)}
            />
          </Row>
        </div>
      </details>
    </section>
  );
}
