import { useEffect, useRef } from 'react';
import { useT } from '@/i18n';
import type { LogEntry } from '@/types';
import { LogItem } from './LogItem';

interface LogPanelProps {
  logs: LogEntry[];
}

/** Scrolling view of backend events, pinned to the newest line. */
export function LogPanel({ logs }: LogPanelProps) {
  const t = useT();
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logs]);

  return (
    <section className="flex h-[650px] flex-col rounded-2xl border border-gray-800 bg-[#0e1015] shadow-2xl">
      <div className="flex items-center justify-between border-b border-gray-800 bg-black/20 p-4">
        <h2 className="text-[9px] font-black uppercase tracking-[0.2em] text-gray-500">
          {t('logs.title')}
        </h2>
        <div className="flex items-center gap-2">
          <span className="text-[8px] font-black uppercase text-gray-600">{t('logs.live')}</span>
          <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-orange-500" />
        </div>
      </div>

      <div
        ref={scrollRef}
        className="custom-scrollbar flex-1 space-y-1 overflow-y-auto scroll-smooth p-4"
      >
        {logs.length === 0 ? (
          <div className="text-[10px] italic text-gray-700">{t('logs.empty')}</div>
        ) : (
          logs.map((log, index) => (
            <LogItem key={`${log.timestamp}-${index}`} log={log} />
          ))
        )}
      </div>
    </section>
  );
}
