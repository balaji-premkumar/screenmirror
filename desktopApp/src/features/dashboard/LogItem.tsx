import { memo } from "react";
import { LogEntry } from "@/types";

interface LogItemProps {
  log: LogEntry;
}

export const LogItem = memo(({ log }: LogItemProps) => {
  const getLevelColor = (level: string) => {
    switch (level) {
      case 'ERROR': case 'FATAL': return 'text-red-400';
      case 'WARN': return 'text-yellow-400';
      case 'SUCCESS': return 'text-green-400';
      default: return 'text-blue-400';
    }
  };

  return (
    <div className="border-l border-gray-800 pl-3 py-0.5 hover:bg-white/5 text-[10px] font-mono flex gap-2">
      <span className="text-gray-600 min-w-[80px]">{log.timestamp}</span>
      <span className={`${getLevelColor(log.level)} font-bold w-[70px]`}>[{log.level}]</span>
      <span className="text-gray-300 break-all">{log.message}</span>
    </div>
  );
});