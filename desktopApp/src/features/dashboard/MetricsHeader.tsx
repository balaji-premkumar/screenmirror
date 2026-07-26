import { useT } from '@/i18n';
import type { Metrics } from '@/types';
import type { ReactNode } from 'react';

interface MetricsHeaderProps {
  metrics: Metrics;
  isConnected: boolean;
  children?: ReactNode;
}

/** Buffer fill above this is healthy; below the lower bound it is failing. */
const BUFFER_HEALTHY = 0.6;
const BUFFER_DEGRADED = 0.3;

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="text-right">
      <div className="mb-1 text-[9px] font-bold uppercase tracking-widest text-gray-500">
        {label}
      </div>
      <div className="text-xl font-black">{children}</div>
    </div>
  );
}

/** Title block plus the live throughput, frame rate and buffer readouts. */
export function MetricsHeader({ metrics, isConnected, children }: MetricsHeaderProps) {
  const t = useT();

  const bufferColour = !isConnected
    ? 'text-gray-700'
    : metrics.buffer_health > BUFFER_HEALTHY
      ? 'text-green-400'
      : metrics.buffer_health > BUFFER_DEGRADED
        ? 'text-yellow-400'
        : 'text-red-400';

  return (
    <header className="flex items-end justify-between border-b border-gray-800 pb-6">
      <div>
        <h1 className="bg-gradient-to-r from-orange-400 to-orange-600 bg-clip-text text-4xl font-black uppercase tracking-tighter text-transparent">
          {t('app.title')}
        </h1>
        <p className="mt-1 font-mono text-[10px] uppercase tracking-[0.3em] text-gray-500">
          {t('app.subtitle')}
        </p>
      </div>

      <div className="flex items-center gap-8">
        <Stat label={t('metrics.throughput')}>
          <span className={metrics.throughput_mbps > 0 ? 'text-green-400' : 'text-gray-700'}>
            {metrics.throughput_mbps.toFixed(2)}{' '}
            <span className="text-[10px]">{t('metrics.throughput.unit')}</span>
          </span>
        </Stat>

        <Stat label={t('metrics.framerate')}>
          <span className={metrics.fps_actual > 0 ? 'text-blue-400' : 'text-gray-700'}>
            {metrics.fps_actual.toFixed(1)}{' '}
            <span className="text-[10px]">{t('metrics.framerate.unit')}</span>
          </span>
        </Stat>

        <Stat label={t('metrics.buffer')}>
          <span className={bufferColour}>
            {(metrics.buffer_health * 100).toFixed(0)}
            <span className="text-[10px]">%</span>
            {metrics.frames_dropped > 0 && (
              <span className="ml-2 text-[9px] text-red-400">
                {t('metrics.dropped', { count: metrics.frames_dropped })}
              </span>
            )}
          </span>
        </Stat>

        <div className="flex gap-2">{children}</div>
      </div>
    </header>
  );
}
