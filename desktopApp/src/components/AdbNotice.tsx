import { useT } from '@/i18n';

/**
 * Shown while a plugged-in phone has USB debugging on.
 *
 * Mirror needs neither ADB nor developer options. With them on, the phone
 * exposes an ADB interface that Android's own ADB drivers also claim, and the
 * accessory comes back as accessory + ADB, which is where drivers end up
 * mismatched. Shared by the loader screen and the dashboard.
 */
export function AdbNotice() {
  const t = useT();
  return (
    <div role="status" className="rounded-2xl border border-warn/40 bg-warn/10 px-5 py-4">
      <div className="font-semibold">{t('adb.title')}</div>
      <div className="mt-0.5 text-sm text-muted">{t('adb.body')}</div>
    </div>
  );
}
