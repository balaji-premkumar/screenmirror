import { useEffect, useRef, useState } from 'react';
import { useI18n, useT } from '@/i18n';
import { renderLogMessage } from '@/i18n/events';
import type { LogEntry } from '@/types';
import { LogItem } from './LogItem';

interface LogPanelProps {
  logs: LogEntry[];
}

/**
 * A single line with the latest event, which opens into the full history.
 *
 * Most sessions never need the history, so it does not take screen space until
 * asked for. The line still changes when something happens, which is enough to
 * notice a problem and open it.
 */
export function LogPanel({ logs }: LogPanelProps) {
  const t = useT();
  const { locale } = useI18n();
  const [open, setOpen] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (open && el) el.scrollTop = el.scrollHeight;
  }, [logs, open]);

  const latest = logs[logs.length - 1];

  return (
    <section aria-label={t('logs.title')} className="border-t border-line pt-3">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full items-center gap-3 rounded-lg py-1 text-left"
      >
        <span className="text-sm font-semibold">{t('logs.title')}</span>
        <span className="min-w-0 flex-1 truncate text-sm text-muted">
          {latest ? renderLogMessage(latest, locale) : t('logs.empty')}
        </span>
        <span
          aria-hidden="true"
          className={`text-muted transition-transform ${open ? 'rotate-90' : ''}`}
        >
          ›
        </span>
      </button>

      {open && (
        <div
          ref={scrollRef}
          className="custom-scrollbar mt-2 max-h-72 space-y-0.5 overflow-y-auto rounded-xl border border-line bg-panel p-3"
        >
          {logs.length === 0 ? (
            <div className="text-sm text-muted">{t('logs.empty')}</div>
          ) : (
            logs.map((log, index) => <LogItem key={`${log.timestamp}-${index}`} log={log} />)
          )}
        </div>
      )}
    </section>
  );
}
