import { useEffect, useState } from 'react';
import { AdbNotice } from '@/components/AdbNotice';
import LogoIcon from '@/components/ui/LogoIcon';
import { useT } from '@/i18n';
import { installUsbAccess } from '@/services/driver';
import { call, isReady } from '@/services/rpc';
import { DriverInstallState, type CheckItem, type CheckStatus, type StartupChecks } from '@/types';

interface LoaderScreenProps {
  onComplete: (checks: StartupChecks) => void;
}

/** Index of each check, so the update calls below read as something. */
const DRIVER = 0;
const PLAYER = 1;
const OBS = 2;
const PLUGIN = 3;

const INITIAL_CHECKS: CheckItem[] = [
  { labelKey: 'loader.check.driver', status: 'pending', detail: '' },
  { labelKey: 'loader.check.player', status: 'pending', detail: '' },
  { labelKey: 'loader.check.obs', status: 'pending', detail: '' },
  { labelKey: 'loader.check.plugin', status: 'pending', detail: '' },
];

const FAILED_CHECKS: StartupChecks = {
  driverOk: false,
  ffplayOk: false,
  obsInstalled: false,
  obsPluginInstalled: false,
  obsPluginDir: '',
  adbEnabled: false,
};

/** Paced so each result is legible rather than flashing past. */
const STEP_DELAY_MS = 300;

/** How often to look for the RPC bridge before the first check can run. */
const BRIDGE_POLL_MS = 200;

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Waits for Electrobun to install the RPC bridge, or gives up. */
async function waitForBridge(timeoutMs = 10_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (isReady()) return true;
    await delay(BRIDGE_POLL_MS);
  }
  return false;
}

const STATUS_ICON: Record<CheckStatus, JSX.Element> = {
  pending: <div className="h-5 w-5 rounded-full border-2 border-line" />,
  checking: (
    <div className="h-5 w-5 rounded-full border-2 border-signal border-t-transparent motion-safe:animate-spin" />
  ),
  ok: (
    <div className="flex h-5 w-5 items-center justify-center rounded-full bg-good text-xs font-bold text-paper">
      ✓
    </div>
  ),
  warn: (
    <div className="flex h-5 w-5 items-center justify-center rounded-full bg-warn text-xs font-bold text-paper">
      !
    </div>
  ),
  error: (
    <div className="flex h-5 w-5 items-center justify-center rounded-full bg-bad text-xs font-bold text-paper">
      ✕
    </div>
  ),
};

const STATUS_COLOUR: Record<CheckStatus, string> = {
  pending: 'text-muted',
  checking: 'text-muted',
  ok: 'text-muted',
  warn: 'text-warn',
  error: 'text-bad',
};

/** Startup screen: reports what the app found, and offers to fix what it can. */
export function LoaderScreen({ onComplete }: LoaderScreenProps) {
  const t = useT();
  const [checks, setChecks] = useState<CheckItem[]>(INITIAL_CHECKS);
  const [startupData, setStartupData] = useState<StartupChecks | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installingDriver, setInstallingDriver] = useState(false);
  const [progress, setProgress] = useState(0);

  const update = (index: number, status: CheckStatus, detail: string) =>
    setChecks((prev) => prev.map((c, i) => (i === index ? { ...c, status, detail } : c)));

  /**
   * Installs USB access and reports it on the driver row. Resolves with
   * whether access is now in place.
   *
   * The install raises an elevation prompt and then waits on the system's
   * driver installer, so the row shows a spinner for as long as that takes.
   */
  const installDriver = async (): Promise<boolean> => {
    setInstallingDriver(true);
    update(DRIVER, 'checking', t('loader.check.driver.installing'));
    try {
      const state = await installUsbAccess();
      if (state === DriverInstallState.Installed) {
        update(DRIVER, 'ok', t('loader.check.driver.installed'));
        return true;
      }
      if (state === DriverInstallState.NotNeeded) {
        // Every package is in place but a phone still would not open: it was
        // plugged in before its driver existed.
        update(DRIVER, 'warn', t('loader.check.driver.replug'));
        return true;
      }
      update(DRIVER, 'error', t('loader.check.driver.failed'));
      return false;
    } catch (error) {
      console.error('Installing USB access failed', error);
      update(DRIVER, 'error', t('loader.check.driver.failed'));
      return false;
    } finally {
      setInstallingDriver(false);
    }
  };

  const retryDriver = async () => {
    const ok = await installDriver();
    setStartupData((prev) => (prev ? { ...prev, driverOk: ok } : prev));
  };

  useEffect(() => {
    let cancelled = false;

    const run = async () => {
      if (!(await waitForBridge()) || cancelled) {
        if (!cancelled) {
          INITIAL_CHECKS.forEach((_, i) => update(i, 'warn', t('loader.check.failed')));
          setStartupData(FAILED_CHECKS);
          setProgress(100);
        }
        return;
      }

      try {
        update(DRIVER, 'checking', t('loader.check.driver.scanning'));
        const data = await call('getStartupChecks');
        if (cancelled) return;

        setProgress(25);
        // The install runs alongside the remaining checks; the screen waits
        // for it before offering the dashboard.
        let driverDone: Promise<boolean>;
        if (data.driverOk) {
          update(DRIVER, 'ok', t('loader.check.driver.ok'));
          driverDone = Promise.resolve(true);
        } else {
          driverDone = installDriver();
        }

        await delay(STEP_DELAY_MS);
        if (cancelled) return;
        setProgress(50);
        update(
          PLAYER,
          data.ffplayOk ? 'ok' : 'warn',
          data.ffplayOk ? t('loader.check.player.ok') : t('loader.check.player.warn'),
        );

        await delay(STEP_DELAY_MS);
        if (cancelled) return;
        setProgress(75);
        update(
          OBS,
          data.obsInstalled ? 'ok' : 'warn',
          data.obsInstalled ? t('loader.check.obs.ok') : t('loader.check.obs.warn'),
        );

        await delay(STEP_DELAY_MS);
        if (cancelled) return;
        setProgress(100);
        if (!data.obsInstalled) {
          update(PLUGIN, 'warn', t('loader.check.plugin.skipped'));
        } else {
          update(
            PLUGIN,
            data.obsPluginInstalled ? 'ok' : 'warn',
            data.obsPluginInstalled
              ? t('loader.check.plugin.ok', { path: data.obsPluginDir })
              : t('loader.check.plugin.warn'),
          );
        }

        const driverOk = await driverDone;
        if (cancelled) return;
        setStartupData({ ...data, driverOk });
      } catch (error) {
        console.error('Startup checks failed', error);
        if (cancelled) return;
        INITIAL_CHECKS.forEach((_, i) => update(i, 'warn', t('loader.check.failed')));
        setStartupData(FAILED_CHECKS);
        setProgress(100);
      }
    };

    void run();
    return () => {
      cancelled = true;
    };
    // `t` is stable for a given locale, and re-running the checks on a language
    // change would restart the whole startup sequence.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const installPlugin = async () => {
    setInstalling(true);
    try {
      const result = await call('installObsPlugin');
      if (result.success) {
        update(PLUGIN, 'ok', t('loader.check.plugin.installed'));
        setStartupData((prev) => (prev ? { ...prev, obsPluginInstalled: true } : prev));
      } else {
        update(
          PLUGIN,
          'error',
          t('loader.check.plugin.failed', { error: result.error ?? '' }),
        );
      }
    } catch (error) {
      update(
        PLUGIN,
        'error',
        t('loader.check.plugin.failed', { error: String(error) }),
      );
    } finally {
      setInstalling(false);
    }
  };

  const done = startupData !== null && !installingDriver;
  const canRetryDriver = startupData !== null && checks[DRIVER].status === 'error';
  const canInstallPlugin =
    checks[PLUGIN].status === 'warn' &&
    startupData?.obsInstalled === true &&
    startupData?.obsPluginInstalled === false;

  // Everything is fine: there is nothing to read, so do not make the person
  // click through. Any warning keeps the screen up until they have seen it.
  const allClear =
    done && !startupData?.adbEnabled && checks.every((c) => c.status === 'ok');
  useEffect(() => {
    if (!allClear || !startupData) return;
    const timer = setTimeout(() => onComplete(startupData), 900);
    return () => clearTimeout(timer);
  }, [allClear, startupData, onComplete]);

  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <div className="w-full max-w-md">
        <div className="mb-8 flex flex-col items-center text-center">
          <LogoIcon size={44} />
          <h1 className="mt-4 text-3xl font-semibold tracking-tight">{t('app.title')}</h1>
          <p className="mt-1 text-muted">{t('loader.initializing')}</p>
        </div>

        <div
          className="mb-6 h-1 overflow-hidden rounded-full bg-line"
          role="progressbar"
          aria-valuenow={progress}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className="h-full rounded-full bg-signal transition-[width] duration-500 ease-out"
            style={{ width: `${progress}%` }}
          />
        </div>

        <ul className="mb-6 divide-y divide-line rounded-2xl border border-line bg-panel">
          {checks.map((check, i) => (
            <li key={check.labelKey} className="flex items-start gap-4 px-5 py-4">
              <div className="mt-0.5 shrink-0">{STATUS_ICON[check.status]}</div>
              <div className="min-w-0 flex-1">
                <div className={`font-semibold ${check.status === 'pending' ? 'text-muted' : ''}`}>
                  {t(check.labelKey)}
                </div>
                {check.detail && (
                  <div className={`mt-0.5 text-sm ${STATUS_COLOUR[check.status]}`}>{check.detail}</div>
                )}
              </div>

              {i === DRIVER && canRetryDriver && (
                <button
                  type="button"
                  onClick={retryDriver}
                  disabled={installingDriver}
                  className="shrink-0 rounded-lg border border-line px-3 py-1.5 text-sm font-semibold transition-colors hover:border-ink/40 disabled:opacity-60"
                >
                  {t('loader.retry')}
                </button>
              )}

              {i === PLUGIN && canInstallPlugin && (
                <button
                  type="button"
                  onClick={installPlugin}
                  disabled={installing}
                  className="shrink-0 rounded-lg border border-line px-3 py-1.5 text-sm font-semibold transition-colors hover:border-ink/40 disabled:opacity-60"
                >
                  {installing ? t('loader.installing') : t('loader.install')}
                </button>
              )}
            </li>
          ))}
        </ul>

        {startupData?.adbEnabled && (
          <div className="mb-6">
            <AdbNotice />
          </div>
        )}

        {!allClear && (
          <div className="flex justify-center">
            <button
              type="button"
              onClick={() => startupData && onComplete(startupData)}
              disabled={!done}
              className="rounded-xl bg-signal px-8 py-3 text-base font-semibold text-signal-ink transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:bg-line disabled:text-muted"
            >
              {done ? t('loader.enter') : t('loader.waiting')}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
