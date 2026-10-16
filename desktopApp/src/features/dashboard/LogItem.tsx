import { memo } from 'react';
import { useI18n } from '@/i18n';
import { renderLogMessage } from '@/i18n/events';
import type { LogEntry, LogLevel } from '@/types';

const LEVEL_COLOUR: Record<LogLevel, string> = {
  FATAL: 'text-bad',
  ERROR: 'text-bad',
  WARN: 'text-warn',
  SUCCESS: 'text-good',
  INFO: 'text-muted',
};

interface LogItemProps {
  log: LogEntry;
}

/**
 * One line in the activity history.
 *
 * The text comes from the event catalog keyed on `log.code`, not from
 * `log.message` — that is what makes these lines translatable. `log.message`
 * is the English the backend already rendered, used only when this build has
 * no entry for the code.
 */
export const LogItem = memo(function LogItem({ log }: LogItemProps) {
  const { locale } = useI18n();
  const message = renderLogMessage(log, locale);

  return (
    <div className="flex gap-3 rounded px-1 py-0.5 text-sm hover:bg-paper">
      <span className="num w-20 shrink-0 text-muted">{log.timestamp}</span>
      <span className={`w-16 shrink-0 font-semibold ${LEVEL_COLOUR[log.level] ?? 'text-muted'}`}>
        {log.level.toLowerCase()}
      </span>
      <span className="break-words">{message}</span>
    </div>
  );
});
