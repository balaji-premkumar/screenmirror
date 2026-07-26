import { useEffect, useState } from 'react';
import LogoIcon from '@/components/ui/LogoIcon';
import { useT } from '@/i18n';
import { call, isReady } from '@/services/rpc';
import type { CheckItem, CheckStatus, StartupChecks } from '@/types';

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
  pending: <div className="h-4 w-4 rounded-full border-2 border-gray-700" />,
  checking: (
    <div className="h-4 w-4 animate-spin rounded-full border-2 border-orange-400 border-t-transparent" />
  ),
  ok: (
    <div className="flex h-4 w-4 items-center justify-center rounded-full bg-green-500 text-[8px] font-black text-black">
      ✓
    </div>
  ),
  warn: (
    <div className="flex h-4 w-4 items-center justify-center rounded-full bg-yellow-500/80 text-[8px] font-black text-black">
      !
    </div>
  ),
  error: (
    <div className="flex h-4 w-4 items-center justify-center rounded-full bg-red-500 text-[8px] font-black text-white">
      ✕
    </div>
  ),
};

const STATUS_COLOUR: Record<CheckStatus, string> = {
  pending: 'text-gray-600',
  checking: 'text-orange-400',
  ok: 'text-green-400',
  warn: 'text-yellow-400',
  error: 'text-red-400',
};

/** Startup screen: reports what the app found, and offers to fix what it can. */
export function LoaderScreen({ onComplete }: LoaderScreenProps) {
  const t = useT();
  const [checks, setChecks] = useState<CheckItem[]>(INITIAL_CHECKS);
  const [startupData, setStartupData] = useState<StartupChecks | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState(0);

  const update = (index: number, status: CheckStatus, detail: string) =>
    setChecks((prev) => prev.map((c, i) => (i === index ? { ...c, status, detail } : c)));

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
        update(
          DRIVER,
          data.driverOk ? 'ok' : 'warn',
          data.driverOk ? t('loader.check.driver.ok') : t('loader.check.driver.warn'),
        );

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

        setStartupData(data);
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

  const done = startupData !== null;
  const canInstallPlugin =
    checks[PLUGIN].status === 'warn' &&
    startupData?.obsInstalled === true &&
    startupData?.obsPluginInstalled === false;

  return (
    <div className="flex min-h-screen items-center justify-center bg-[#050608] p-6">
      <div className="w-full max-w-lg">
        <div className="mb-10 text-center">
          <div className="mb-6 inline-flex items-center justify-center">
            <div className="relative">
              <div className="absolute inset-0 animate-pulse rounded-full bg-orange-500/20 blur-2xl" />
              <LogoIcon />
            </div>
          </div>
          <h1 className="mb-2 bg-gradient-to-r from-orange-400 to-orange-600 bg-clip-text text-3xl font-black uppercase tracking-tighter text-transparent">
            {t('app.title')}
          </h1>
          <p className="font-mono text-[10px] uppercase tracking-[0.4em] text-gray-500">
            {t('loader.initializing')}
          </p>
        </div>

        <div className="mb-8 px-4">
          <div className="h-[2px] overflow-hidden rounded-full bg-gray-800">
            <div
              className="h-full rounded-full bg-gradient-to-r from-orange-500 to-orange-400 transition-all duration-700 ease-out"
              style={{ width: `${progress}%` }}
            />
          </div>
        </div>

        <div className="mb-6 space-y-1 rounded-2xl border border-gray-800/50 bg-[#0a0c10] p-6 shadow-2xl">
          {checks.map((check, i) => (
            <div
              key={check.labelKey}
              className={`flex items-start gap-4 rounded-xl px-3 py-3 transition-all duration-300 ${
                check.status === 'checking' ? 'bg-orange-500/5' : ''
              }`}
            >
              <div className="mt-0.5 flex-shrink-0">{STATUS_ICON[check.status]}</div>
              <div className="min-w-0 flex-1">
                <div
                  className={`text-xs font-bold transition-colors duration-300 ${
                    check.status === 'pending' ? 'text-gray-600' : 'text-gray-200'
                  }`}
                >
                  {t(check.labelKey)}
                </div>
                {check.detail && (
                  <div
                    className={`mt-0.5 truncate text-[10px] font-medium ${STATUS_COLOUR[check.status]}`}
                  >
                    {check.detail}
                  </div>
                )}
              </div>

              {i === PLUGIN && canInstallPlugin && (
                <button
                  type="button"
                  onClick={installPlugin}
                  disabled={installing}
                  className="flex-shrink-0 cursor-pointer rounded-lg border border-blue-500/20 bg-blue-500/10 px-3 py-1.5 text-[9px] font-black uppercase text-blue-400 transition-all hover:bg-blue-500/20"
                >
                  {installing ? t('loader.installing') : t('loader.install')}
                </button>
              )}
            </div>
          ))}
        </div>

        <div className="flex justify-center">
          <button
            type="button"
            onClick={() => startupData && onComplete(startupData)}
            disabled={!done}
            className={`cursor-pointer rounded-xl px-12 py-3 text-xs font-black uppercase tracking-widest transition-all duration-500 ${
              done
                ? 'bg-gradient-to-r from-orange-500 to-orange-600 text-black shadow-xl shadow-orange-900/30 hover:scale-[1.02] hover:shadow-orange-600/40 active:scale-[0.98]'
                : 'cursor-not-allowed bg-gray-800 text-gray-600'
            }`}
          >
            {done ? t('loader.enter') : t('loader.waiting')}
          </button>
        </div>

        <p className="mt-8 text-center text-[9px] uppercase tracking-widest text-gray-700">
          {t('app.version')}
        </p>
      </div>
    </div>
  );
}
