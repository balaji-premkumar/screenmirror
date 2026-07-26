import { useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';

interface StatusBarProps {
  driverOk: boolean;
}

/**
 * The strip along the top: engine state, plus the button that requests USB
 * permissions when they are missing.
 *
 * The button raises an elevation prompt (pkexec or UAC), so it only appears
 * when there is actually something to install.
 */
export function StatusBar({ driverOk }: StatusBarProps) {
  const t = useT();
  const [isRepairing, setIsRepairing] = useState(false);

  const repair = async () => {
    if (isRepairing) return;
    setIsRepairing(true);
    try {
      await call('repairDrivers');
    } catch (error) {
      console.error('Repairing USB permissions failed', error);
    } finally {
      setIsRepairing(false);
    }
  };

  return (
    <div
      className={`flex items-center justify-between rounded-xl border px-4 py-2 ${
        driverOk
          ? 'border-green-500/20 bg-green-900/10 text-green-400'
          : 'border-yellow-500/20 bg-yellow-900/10 text-yellow-400'
      }`}
    >
      <div className="flex items-center gap-2 text-[10px] font-black uppercase tracking-widest">
        <span
          className={`h-1.5 w-1.5 rounded-full ${
            driverOk ? 'bg-green-400' : 'animate-pulse bg-yellow-400'
          }`}
        />
        {driverOk ? t('status.engine.ok') : t('status.engine.needsDriver')}
      </div>

      {!driverOk && (
        <button
          type="button"
          onClick={repair}
          disabled={isRepairing}
          className={`cursor-pointer rounded bg-yellow-500 px-3 py-1 text-[10px] font-black uppercase text-black transition-all hover:bg-white ${
            isRepairing ? 'cursor-wait opacity-50' : ''
          }`}
        >
          {isRepairing ? t('status.repairing') : t('status.repair')}
        </button>
      )}
    </div>
  );
}
