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
// A value import, so relative: Electrobun's bundler does not read the `@/`
// alias from tsconfig. The type-only imports above are erased before bundling.
import { DriverInstallState } from '../types';
import { native } from './native';

const ZERO_METRICS: Metrics = {
  throughput_mbps: 0,
  decode_latency_ms: 0,
  link_delay_ms: 0,
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
    adbEnabled: native.adbEnabled(),
  }),

  getStartupChecks: (): StartupChecks => ({
    driverOk: native.driverOk(),
    ffplayOk: native.ffplayAvailable(),
    obsInstalled: native.obsInstalled(),
    obsPluginInstalled: native.obsPluginInstalled(),
    obsPluginDir: native.obsPluginDir(),
    adbEnabled: native.adbEnabled(),
  }),

  // The install runs on a backend thread: it waits on an elevation prompt and
  // then on the system's driver installer, and doing that inside a handler
  // froze the event loop, so the interface could not show any progress.
  // macOS needs nothing: IOKit lets a user-space process claim an interface
  // no kernel driver has taken, and none claims an accessory.
  startDriverInstall: (): Ack => ({ success: native.startDriverInstall() }),

  getDriverInstallState: (): DriverInstallState => {
    const state = native.driverInstallState() as DriverInstallState;
    if (state !== DriverInstallState.Running) {
      console.log(`USB driver install state: ${state}`);
    }
    return state;
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
