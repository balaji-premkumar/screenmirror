import type { ReactNode } from 'react';
import { useT } from '@/i18n';
import type { Device } from '@/types';
import { LinkGraphic } from './LinkGraphic';
import type { HealthLevel, StageId } from './stage';
import type { Session } from './useSession';

interface LinkStageProps {
  stage: StageId;
  device: Device | null;
  fps: number;
  /** Verdict text shown under the headline while mirroring. */
  verdict?: { level: HealthLevel; text: string; hint?: string };
  session: Session;
  /** Playback and OBS buttons; offered once the link is up. */
  sinks: ReactNode;
}

const PRIMARY =
  'rounded-xl bg-signal px-6 py-3 text-base font-semibold text-signal-ink transition-[filter] hover:brightness-110 disabled:cursor-wait disabled:opacity-60';
const SECONDARY =
  'rounded-xl border border-line bg-panel px-4 py-2.5 text-sm font-semibold text-ink transition-colors hover:border-ink/40';

const DOT: Record<HealthLevel, string> = {
  good: 'bg-good',
  warn: 'bg-warn',
  bad: 'bg-bad',
};

/**
 * The top of the dashboard: where the stream is in its life, in a sentence,
 * with the one button that moves it to the next step.
 */
export function LinkStage({ stage, device, fps, verdict, session, sinks }: LinkStageProps) {
  const t = useT();
  const name = device?.name || t('stage.thePhone');

  const title = t(`stage.${stage}.title`, { name, fps: Math.round(fps) });
  const body =
    stage === 'mirroring' && verdict ? verdict.text : t(`stage.${stage}.body`, { name });

  const linked = stage === 'ready' || stage === 'waiting-for-phone' || stage === 'mirroring';

  return (
    <section
      aria-labelledby="stage-title"
      className="rounded-[28px] border border-line bg-panel px-6 pb-7 pt-8 sm:px-10"
    >
      <LinkGraphic stage={stage} fps={fps} label={title} tone={verdict?.level} />

      <div className="mt-6 flex flex-col gap-6">
        <div className="max-w-2xl">
          <h1 id="stage-title" className="text-4xl font-semibold leading-[1.1] tracking-tight">
            {title}
          </h1>
          <p className="mt-3 flex items-start gap-2.5 text-lg leading-snug text-muted" aria-live="polite">
            {stage === 'mirroring' && verdict && (
              <span className={`mt-[0.55em] h-2.5 w-2.5 shrink-0 rounded-full ${DOT[verdict.level]}`} />
            )}
            <span>
              {body}
              {stage === 'mirroring' && verdict?.hint && (
                <span className="block text-base text-muted/90">{verdict.hint}</span>
              )}
            </span>
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-3 empty:hidden">
          {stage === 'phone-found' && device && (
            <button
              type="button"
              className={PRIMARY}
              disabled={session.busy === 'connect'}
              onClick={() => session.connect(device.raw)}
            >
              {session.busy === 'connect' ? t('stage.connecting') : t('stage.connect', { name })}
            </button>
          )}

          {stage === 'ready' && (
            <button
              type="button"
              className={PRIMARY}
              disabled={session.busy === 'start'}
              onClick={session.start}
            >
              {session.busy === 'start' ? t('stage.starting') : t('stage.start')}
            </button>
          )}

          {stage === 'waiting-for-phone' && (
            <button type="button" className={SECONDARY} onClick={session.stop}>
              {t('stage.cancel')}
            </button>
          )}

          {stage === 'mirroring' && (
            <button type="button" className={PRIMARY} onClick={session.stop}>
              {t('stage.stop')}
            </button>
          )}

          {linked && sinks}

          {linked && (
            <button
              type="button"
              className="rounded-xl px-3 py-2.5 text-sm font-semibold text-muted transition-colors hover:text-ink sm:ml-auto"
              onClick={session.disconnect}
            >
              {t('stage.disconnect')}
            </button>
          )}
        </div>
      </div>
    </section>
  );
}
