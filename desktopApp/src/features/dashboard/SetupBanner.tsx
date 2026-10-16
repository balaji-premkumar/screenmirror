import { useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';

/**
 * Shown only when USB access is missing, because that is the one problem the
 * person has to fix before anything else on the page can work.
 *
 * The button raises an elevation prompt (pkexec or UAC), so it never appears
 * when there is nothing to install.
 */
export function SetupBanner() {
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
      role="alert"
      className="flex flex-col gap-3 rounded-2xl border border-warn/40 bg-warn/10 px-5 py-4 sm:flex-row sm:items-center sm:justify-between"
    >
      <div>
        <div className="font-semibold">{t('status.engine.needsDriver')}</div>
        <div className="text-sm text-muted">{t('status.engine.needsDriver.body')}</div>
      </div>
      <button
        type="button"
        onClick={repair}
        disabled={isRepairing}
        className="shrink-0 rounded-xl bg-ink px-4 py-2.5 text-sm font-semibold text-paper transition-opacity hover:opacity-90 disabled:cursor-wait disabled:opacity-60"
      >
        {isRepairing ? t('status.repairing') : t('status.repair')}
      </button>
    </div>
  );
}
