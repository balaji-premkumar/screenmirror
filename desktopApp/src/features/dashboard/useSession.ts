import { useCallback, useEffect, useState } from 'react';
import { call } from '@/services/rpc';
import { parseDevice, type StreamConfig } from '@/types';
import { ACCESSORY, matchPreset, PRESETS } from './stage';

const BALANCED = PRESETS[1];

const DEFAULT_CONFIG: StreamConfig = {
  resolution: BALANCED.resolution,
  bitrate: BALANCED.bitrate,
  fps: BALANCED.fps,
  audioSource: 'Game + Mic',
};

/** How long to wait for the phone's consent prompt before assuming it was dismissed. */
const PROMPT_TIMEOUT_MS = 60_000;

/**
 * Everything the person *does* on the dashboard: pick quality, connect, start,
 * stop. State lives here so the stage (which shows the Start button) and the
 * quality panel (which edits the settings) read the same values.
 */
export function useSession(isConnected: boolean, fps: number) {
  const [config, setConfig] = useState<StreamConfig>(DEFAULT_CONFIG);
  const [startRequested, setStartRequested] = useState(false);
  const [busy, setBusy] = useState<'connect' | 'start' | null>(null);

  const update = useCallback(
    <K extends keyof StreamConfig>(key: K, value: StreamConfig[K]) =>
      setConfig((prev) => ({ ...prev, [key]: value })),
    [],
  );

  const applyPreset = useCallback(
    (id: (typeof PRESETS)[number]['id']) => {
      const preset = PRESETS.find((p) => p.id === id);
      if (preset) {
        setConfig((prev) => ({
          ...prev,
          resolution: preset.resolution,
          bitrate: preset.bitrate,
          fps: preset.fps,
        }));
      }
    },
    [],
  );

  // Frames arriving means the request has been answered; a dropped link means
  // it no longer applies. Either way the flag must not outlive its meaning, or
  // the next session would open on "waiting for your phone".
  useEffect(() => {
    if (fps > 0 || !isConnected) setStartRequested(false);
  }, [fps, isConnected]);

  useEffect(() => {
    if (!startRequested) return;
    const timer = setTimeout(() => setStartRequested(false), PROMPT_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [startRequested]);

  const connect = useCallback(async (raw: string) => {
    setBusy('connect');
    try {
      const { kind, id } = parseDevice(raw);
      if (kind === ACCESSORY) {
        // Already in accessory mode — nothing left to negotiate; re-arm
        // auto-reconnect and let the listener pick it up.
        await call('toggleAutoReconnect', { enabled: true });
      } else {
        const [vid, pid] = id.split(':').map((part) => parseInt(part, 16));
        await call('triggerHandshake', { vid, pid });
      }
    } catch (error) {
      console.error('Connecting failed', error);
    } finally {
      setBusy(null);
    }
  }, []);

  const start = useCallback(async () => {
    setBusy('start');
    try {
      await call('syncConfig', { ...config, command: 'start' });
      setStartRequested(true);
    } catch (error) {
      console.error('Starting capture failed', error);
    } finally {
      setBusy(null);
    }
  }, [config]);

  const stop = useCallback(async () => {
    setStartRequested(false);
    try {
      await call('syncConfig', { command: 'stop' });
    } catch (error) {
      console.error('Stopping capture failed', error);
    }
  }, []);

  const disconnect = useCallback(async () => {
    setStartRequested(false);
    try {
      await call('disconnectDevice');
    } catch (error) {
      console.error('Disconnecting failed', error);
    }
  }, []);

  return {
    config,
    update,
    applyPreset,
    activePreset: matchPreset(config)?.id,
    startRequested,
    busy,
    connect,
    start,
    stop,
    disconnect,
  };
}

export type Session = ReturnType<typeof useSession>;
