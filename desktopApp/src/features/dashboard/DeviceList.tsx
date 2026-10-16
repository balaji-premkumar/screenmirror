import { useT } from '@/i18n';
import type { Device } from '@/types';
import type { Session } from './useSession';

interface OtherDevicesProps {
  /** Every device except the one the stage is already talking about. */
  devices: Device[];
  session: Session;
}

/** Further phones on the bus, offered only when there is more than one. */
export function OtherDevices({ devices, session }: OtherDevicesProps) {
  const t = useT();
  if (devices.length === 0) return null;

  return (
    <section aria-labelledby="others-title">
      <h2 id="others-title" className="text-xl font-semibold tracking-tight">
        {t('devices.others')}
      </h2>
      <ul className="mt-3 divide-y divide-line rounded-2xl border border-line bg-panel">
        {devices.map((device) => (
          <li key={device.raw} className="flex items-center justify-between gap-4 px-4 py-3">
            <div>
              <div className="font-semibold">{device.name || t('stage.thePhone')}</div>
              <div className="num text-sm text-muted">{device.id}</div>
            </div>
            <button
              type="button"
              disabled={session.busy === 'connect'}
              onClick={() => session.connect(device.raw)}
              className="rounded-lg border border-line px-3 py-1.5 text-sm font-semibold transition-colors hover:border-ink/40 disabled:opacity-60"
            >
              {t('devices.connect')}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
