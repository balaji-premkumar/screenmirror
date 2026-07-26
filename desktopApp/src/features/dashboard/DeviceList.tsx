import { useState } from 'react';
import { useT } from '@/i18n';
import { call } from '@/services/rpc';
import { parseDevice } from '@/types';

interface DeviceListProps {
  devices: string[];
  isConnected: boolean;
}

/** The device kind the phone reports once it has entered accessory mode. */
const ACCESSORY = 'Accessory';

/** Discovered USB devices, and the action to connect or disconnect each one. */
export function DeviceList({ devices, isConnected }: DeviceListProps) {
  const t = useT();
  const [connectingId, setConnectingId] = useState<string | null>(null);

  const disconnect = () =>
    call('disconnectDevice').catch((error) =>
      console.error('Disconnecting failed', error),
    );

  const connect = async (raw: string) => {
    if (connectingId) return;
    setConnectingId(raw);
    try {
      const { kind, id } = parseDevice(raw);
      if (kind === ACCESSORY) {
        // Already in accessory mode — there is no handshake left to do, so
        // just re-arm auto-reconnect and let the listener pick it up.
        await call('toggleAutoReconnect', { enabled: true });
      } else {
        const [vid, pid] = id.split(':').map((part) => parseInt(part, 16));
        await call('triggerHandshake', { vid, pid });
      }
    } catch (error) {
      console.error('Connecting failed', error);
    } finally {
      setConnectingId(null);
    }
  };

  return (
    <section className="rounded-2xl border border-gray-800 bg-[#0e1015] p-6 shadow-2xl">
      <h2 className="mb-6 text-[9px] font-black uppercase tracking-[0.2em] text-gray-500">
        {t('devices.title')}
      </h2>

      <div className="space-y-3">
        {devices.length === 0 ? (
          <div className="flex animate-pulse items-center gap-3 py-4 text-sm italic text-gray-600">
            {t('devices.scanning')}
          </div>
        ) : (
          devices.map((raw, index) => {
            const { kind, name, id } = parseDevice(raw);
            const isStreaming = kind === ACCESSORY && isConnected;
            const isConnecting = connectingId === raw;

            return (
              <div
                key={`${id}-${index}`}
                className={`flex items-center justify-between rounded-xl border bg-black/40 p-4 transition-all ${
                  isStreaming
                    ? 'border-green-500/30'
                    : 'border-gray-800/50 hover:border-orange-500/30'
                }`}
              >
                <div className="flex items-center gap-4">
                  <span
                    className={`h-2 w-2 rounded-full ${
                      isStreaming ? 'bg-green-400 shadow-[0_0_10px_#4ade80]' : 'bg-blue-400'
                    }`}
                  />
                  <div>
                    <div className="mb-1 text-sm font-black leading-none text-white">{name}</div>
                    <div className="font-mono text-[9px] font-bold uppercase text-gray-500">
                      {id} // {kind}
                    </div>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={() => (isStreaming ? disconnect() : connect(raw))}
                  disabled={isConnecting}
                  className={`cursor-pointer rounded-lg border px-6 py-2 text-[10px] font-black uppercase transition-all ${
                    isStreaming
                      ? 'border-red-500/20 bg-red-500/10 text-red-400 hover:bg-red-500 hover:text-white'
                      : 'border-orange-500/20 bg-orange-500/10 text-orange-500 hover:bg-orange-500 hover:text-black'
                  } ${isConnecting ? 'cursor-wait opacity-50' : ''}`}
                >
                  {isStreaming
                    ? t('devices.disconnect')
                    : isConnecting
                      ? t('devices.connecting')
                      : t('devices.connect')}
                </button>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}
