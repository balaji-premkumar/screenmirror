import { useT } from '@/i18n';
import type { Metrics } from '@/types';

interface HealthPanelProps {
  metrics: Metrics;
  isMirroring: boolean;
}

function Reading({
  value,
  unit,
  label,
  help,
}: {
  value: string;
  unit: string;
  label: string;
  help: string;
}) {
  return (
    <div title={help}>
      <div className="text-3xl font-semibold tracking-tight">
        {value}
        <span className="ml-1 text-base font-medium text-muted">{unit}</span>
      </div>
      <div className="mt-0.5 text-sm text-muted">{label}</div>
    </div>
  );
}

/** The numbers behind the verdict, for when "smooth" is not enough to go on. */
export function HealthPanel({ metrics, isMirroring }: HealthPanelProps) {
  const t = useT();

  return (
    <section aria-labelledby="health-title">
      <h2 id="health-title" className="text-xl font-semibold tracking-tight">
        {t('health.title')}
      </h2>

      {isMirroring ? (
        <>
          <div className="mt-5 grid grid-cols-3 gap-4">
            <Reading
              value={metrics.fps_actual.toFixed(0)}
              unit={t('metrics.framerate.unit')}
              label={t('health.fps')}
              help={t('health.fps.help')}
            />
            <Reading
              value={metrics.throughput_mbps.toFixed(1)}
              unit={t('metrics.throughput.unit')}
              label={t('health.rate')}
              help={t('health.rate.help')}
            />
            <Reading
              value={String(Math.round(metrics.link_delay_ms))}
              unit="ms"
              label={t('health.delay')}
              help={t('health.delay.help')}
            />
          </div>
          <p className="mt-5 text-sm text-muted">
            {metrics.frames_dropped > 0
              ? t('health.skipped', { count: metrics.frames_dropped })
              : t('health.noneSkipped')}
          </p>
        </>
      ) : (
        <p className="mt-2 text-muted">{t('health.idle')}</p>
      )}
    </section>
  );
}
