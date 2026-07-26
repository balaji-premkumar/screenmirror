/**
 * RPC handlers the renderer calls.
 *
 * Every method here has a matching entry in {@link RpcMethods} in
 * `src/services/rpc.ts`; that interface is what makes a call from React
 * type-checked. Adding a method means adding it in both places.
 *
 * Note what is deliberately *absent*: there is no `stopMirror`. Shutting the
 * backend down tears out the USB listener, the decoder and the shared memory,
 * and nothing in the interface calls `init_mirror` again — a "stop" button
 * wired to it would leave the app inert until restart. Stopping a stream is
 * `syncConfig` with `command: "stop"`, which is what the UI uses.
 */

import { defineElectrobunRPC } from 'electrobun/bun';
import type { Ack, RpcMethods } from '@/services/rpc';
import type { LogEntry, Metrics, StartupChecks, StatusUpdate } from '@/types';
import { native } from './native';

const ZERO_METRICS: Metrics = {
  throughput_mbps: 0,
  decode_latency_ms: 0,
  fps_actual: 0,
  frames_dropped: 0,
  buffer_health: 0,
};

/**
 * Typed so the compiler checks this object against the contract the renderer
 * relies on. Electrobun's own handler type is looser, hence the cast at the
 * point of registration rather than here.
 */
const requests: {
  [M in keyof RpcMethods]: (
    params: Parameters<RpcMethods[M]>[0],
  ) => ReturnType<RpcMethods[M]>;
} = {
  getStatusUpdate: (): StatusUpdate => ({
    bufferSize: native.bufferSize(),
    isActive: native.isStreaming(),
    decoder: 'Hardware decoder',
    devices: native.devices(),
    newLogs: native.newLogs<LogEntry>(),
    metrics: native.metrics<Metrics>(ZERO_METRICS),
    driverOk: native.driverOk(),
    // Cached for a few seconds inside the backend, so polling it is cheap.
    obsRunning: native.obsRunning(),
    playerState: native.playerState() as StatusUpdate['playerState'],
  }),

  getStartupChecks: (): StartupChecks => ({
    driverOk: native.driverOk(),
    ffplayOk: native.ffplayAvailable(),
    obsInstalled: native.obsInstalled(),
    obsPluginInstalled: native.obsPluginInstalled(),
    obsPluginDir: native.obsPluginDir(),
  }),

  repairDrivers: (): number => {
    if (process.platform === 'linux') return native.installLinuxPermissions();
    if (process.platform === 'win32') return native.installWindowsDriver();
    // macOS needs no driver: IOKit lets a user-space process claim an
    // interface no kernel driver has taken, and none claims an accessory.
    return 0;
  },

  installObsPlugin: (): Ack => ({ success: native.installObsPlugin() === 0 }),

  triggerHandshake: ({ vid, pid }) => native.handshake(vid, pid),

  toggleAutoReconnect: ({ enabled }): Ack => {
    native.setAutoReconnect(enabled);
    return { success: true };
  },

  toggleObsFeed: ({ enabled }): Ack => {
    native.setObsFeed(enabled);
    return { success: true };
  },

  syncConfig: (config): Ack => ({ success: native.syncConfig(config) === 0 }),

  disconnectDevice: () => native.disconnect(),

  startPlayer: (): Ack => ({ success: native.startPlayer() === 0 }),

  stopPlayer: (): Ack => {
    native.stopPlayer();
    return { success: true };
  },
};

export const rpc = defineElectrobunRPC('bun', {
  handlers: { requests: requests as never },
});
