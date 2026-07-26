/**
 * Typed client for the Bun process.
 *
 * Every component used to carry its own copy of
 *
 *     window.__mirrorRpc || (window.Electrobun && window.Electrobun.rpc)
 *
 * behind a `@ts-ignore`, then call `rpc.request('someMethod', {...})` with
 * `any` on both sides. Nothing checked that the method name existed or that
 * the payload matched, so a typo failed silently at runtime in whichever
 * feature happened to contain it.
 *
 * {@link RpcMethods} names the contract once. Adding a method here and in
 * `src/bun/rpc.ts` is what makes it callable; anything else is a type error.
 */

import type { StartupChecks, StatusUpdate, StreamConfig } from '@/types';

/** `{ success }` is what every mutating handler in `src/bun/rpc.ts` returns. */
export interface Ack {
  success: boolean;
  error?: string;
}

/**
 * Every RPC the Bun process answers, as `method: (params) => result`.
 *
 * Keep in step with the `handlers.requests` object in `src/bun/rpc.ts`.
 */
export interface RpcMethods {
  getStatusUpdate: () => StatusUpdate;
  getStartupChecks: () => StartupChecks;
  repairDrivers: () => number;
  installObsPlugin: () => Ack;
  triggerHandshake: (params: { vid: number; pid: number }) => number;
  toggleAutoReconnect: (params: { enabled: boolean }) => Ack;
  toggleObsFeed: (params: { enabled: boolean }) => Ack;
  syncConfig: (params: Partial<StreamConfig> & { command: 'start' | 'stop' }) => Ack;
  disconnectDevice: () => number;
  startPlayer: () => Ack;
  stopPlayer: () => Ack;
}

export type RpcMethod = keyof RpcMethods;

type Params<M extends RpcMethod> = Parameters<RpcMethods[M]>[0];
type Result<M extends RpcMethod> = ReturnType<RpcMethods[M]>;

interface RawTransport {
  request: (method: string, data?: unknown) => Promise<unknown>;
}

declare global {
  interface Window {
    __mirrorRpc?: RawTransport;
    Electrobun?: { rpc?: RawTransport };
  }
}

/**
 * The transport, or `null` before Electrobun has installed it.
 *
 * The window is created before the RPC bridge attaches, so React can mount
 * and poll for a few hundred milliseconds while this is still null. Callers
 * treat that as "not ready yet", not as an error.
 */
function transport(): RawTransport | null {
  if (typeof window === 'undefined') return null;
  return window.__mirrorRpc ?? window.Electrobun?.rpc ?? null;
}

/** Whether the bridge is up. */
export function isReady(): boolean {
  return transport() !== null;
}

/** Raised when a call is made before the bridge exists. */
export class RpcNotReadyError extends Error {
  constructor(method: string) {
    super(`RPC bridge not ready; "${method}" was called too early`);
    this.name = 'RpcNotReadyError';
  }
}

/**
 * Calls a backend method.
 *
 * @throws {RpcNotReadyError} before the bridge is installed.
 */
export async function call<M extends RpcMethod>(
  ...[method, params]: Params<M> extends undefined ? [M] : [M, Params<M>]
): Promise<Result<M>> {
  const bridge = transport();
  if (!bridge) throw new RpcNotReadyError(method);
  return (await bridge.request(method, params)) as Result<M>;
}

/**
 * Calls a backend method, returning `fallback` instead of throwing.
 *
 * For polling and for fire-and-forget buttons, where an unhandled rejection
 * every 500 ms while the bridge starts up is noise rather than information.
 */
export async function tryCall<M extends RpcMethod>(
  ...args: Params<M> extends undefined
    ? [M, { fallback: Result<M> }]
    : [M, Params<M>, { fallback: Result<M> }]
): Promise<Result<M>> {
  const method = args[0] as M;
  const hasParams = args.length === 3;
  const params = hasParams ? (args[1] as Params<M>) : undefined;
  const fallback = (hasParams ? args[2] : args[1]) as { fallback: Result<M> };

  const bridge = transport();
  if (!bridge) return fallback.fallback;
  try {
    return (await bridge.request(method, params)) as Result<M>;
  } catch (error) {
    console.error(`RPC "${method}" failed`, error);
    return fallback.fallback;
  }
}
