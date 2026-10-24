/**
 * A stand-in for the Bun process, so the interface can be developed and
 * reviewed in a plain browser without a phone or the Rust library.
 *
 * Loaded only in `vite` dev mode and only when the URL has `?mock`
 * (optionally `?mock=ready`). Production builds never include it: the import
 * in `main.tsx` is behind `import.meta.env.DEV`, which Vite folds to `false`.
 *
 * Pressing the real buttons walks the same sequence a phone would: connect,
 * start, a few seconds waiting on the consent prompt, then video.
 */

import type { LogEntry, Metrics, PlayerState, StartupChecks, StatusUpdate } from '@/types';

type Scene = 'none' | 'found' | 'ready' | 'waiting' | 'mirroring' | 'struggling';

const SCENES: Scene[] = ['none', 'found', 'ready', 'waiting', 'mirroring', 'struggling'];

const STARTUP: StartupChecks = {
  driverOk: true,
  ffplayOk: true,
  obsInstalled: true,
  obsPluginInstalled: true,
  obsPluginDir: '~/.config/obs-studio/plugins',
  adbEnabled: false,
};

const PHONE_BEFORE = 'Phone|Pixel 8|18d1:4ee1';
const PHONE_LINKED = 'Accessory|Pixel 8|18d1:2d01';

export function installMockBridge(initial: Scene = 'none') {
  let scene: Scene = SCENES.includes(initial) ? initial : 'none';
  let driverOk = true;
  let player = 0 as PlayerState;
  let pendingLogs: LogEntry[] = [];
  let waitingTimer: ReturnType<typeof setTimeout> | undefined;

  const log = (code: string, level: LogEntry['level'], message: string) =>
    pendingLogs.push({
      timestamp: new Date().toLocaleTimeString([], { hour12: false }),
      code,
      level,
      component: 'USB',
      action: 'streaming',
      params: {},
      message,
    });

  const setScene = (next: Scene) => {
    scene = next;
    select.value = next;
    if (next === 'ready') log('usb.streaming.link_established', 'SUCCESS', 'Connected to the phone. Interface 0 claimed.');
    if (next === 'mirroring') log('demux.frame.first', 'SUCCESS', 'First video frame received.');
  };

  const metrics = (): Metrics => {
    const wobble = Math.sin(Date.now() / 900);
    if (scene === 'mirroring')
      return {
        throughput_mbps: 12.4 + wobble * 0.8,
        decode_latency_ms: 4,
        link_delay_ms: Math.round(9 + wobble * 4),
        fps_actual: 59.6 + wobble * 0.4,
        frames_dropped: 0,
        buffer_health: 0.96,
      };
    if (scene === 'struggling')
      return {
        throughput_mbps: 18.9 + wobble * 3,
        decode_latency_ms: 11,
        link_delay_ms: 260 + Math.round(wobble * 40),
        fps_actual: 41 + wobble * 5,
        frames_dropped: 134,
        buffer_health: 0.22,
      };
    return { throughput_mbps: 0, decode_latency_ms: 0, link_delay_ms: 0, fps_actual: 0, frames_dropped: 0, buffer_health: 0 };
  };

  const devices = () =>
    scene === 'none' ? [] : scene === 'found' ? [PHONE_BEFORE] : [PHONE_LINKED];

  const handlers: Record<string, (data?: any) => unknown> = {
    getStartupChecks: () => STARTUP,
    getStatusUpdate: (): StatusUpdate => {
      const newLogs = pendingLogs;
      pendingLogs = [];
      return {
        bufferSize: 0,
        isActive: scene !== 'none' && scene !== 'found',
        decoder: 'VAAPI',
        devices: devices(),
        newLogs,
        metrics: metrics(),
        driverOk,
        obsRunning: true,
        playerState: player,
        adbEnabled: false,
      };
    },
    triggerHandshake: () => {
      log('aoa.handshake.switching', 'SUCCESS', 'Switching the device to accessory mode…');
      setTimeout(() => setScene('ready'), 800);
      return 0;
    },
    toggleAutoReconnect: () => ({ success: true }),
    syncConfig: (data: { command: 'start' | 'stop' | 'options' }) => {
      clearTimeout(waitingTimer);
      if (data.command === 'start') {
        log('usb.streaming.config_sent', 'SUCCESS', 'Settings sent (96 bytes).');
        // The phone's consent prompt: nothing flows until it is approved.
        scene = 'ready';
        waitingTimer = setTimeout(() => setScene('mirroring'), 4000);
      } else {
        setScene('ready');
      }
      return { success: true };
    },
    disconnectDevice: () => {
      clearTimeout(waitingTimer);
      setScene('found');
      return 0;
    },
    startPlayer: () => ((player = 2 as PlayerState), { success: true }),
    stopPlayer: () => ((player = 0 as PlayerState), { success: true }),
    toggleObsFeed: () => ({ success: true }),
    startDriverInstall: () => ((driverOk = true), { success: true }),
    getDriverInstallState: () => 2,
    installObsPlugin: () => ({ success: true }),
  };

  (window as any).__mirrorRpc = {
    request: async (method: string, data?: unknown) => handlers[method]?.(data),
  };

  // Scene picker, for reviewing states without clicking through to them.
  const select = document.createElement('select');
  select.setAttribute('aria-label', 'Preview scene');
  select.style.cssText =
    'position:fixed;right:12px;bottom:12px;z-index:9999;font:12px system-ui;padding:4px 8px;border-radius:8px;opacity:.85';
  for (const s of [...SCENES, 'driver-missing']) select.add(new Option(`preview: ${s}`, s));
  select.value = scene;
  select.onchange = () => {
    if (select.value === 'driver-missing') driverOk = false;
    else {
      driverOk = true;
      clearTimeout(waitingTimer);
      setScene(select.value as Scene);
    }
  };
  document.body.appendChild(select);
}
