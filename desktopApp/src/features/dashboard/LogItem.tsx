import { memo } from 'react';
import { useI18n } from '@/i18n';
import { renderLogMessage } from '@/i18n/events';
import type { LogEntry, LogLevel } from '@/types';

const LEVEL_COLOUR: Record<LogLevel, string> = {
  FATAL: 'text-red-400',
  ERROR: 'text-red-400',
  WARN: 'text-yellow-400',
  SUCCESS: 'text-green-400',
  INFO: 'text-blue-400',
};

interface LogItemProps {
  log: LogEntry;
}

/**
 * One line in the activity panel.
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
    <div className="flex gap-2 border-l border-gray-800 py-0.5 pl-3 font-mono text-[10px] hover:bg-white/5">
      <span className="min-w-[80px] text-gray-600">{log.timestamp}</span>
      <span className={`w-[70px] font-bold ${LEVEL_COLOUR[log.level] ?? 'text-blue-400'}`}>
        [{log.level}]
      </span>
      <span className="break-all text-gray-300">{message}</span>
    </div>
  );
});
