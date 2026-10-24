/**
 * Installing USB access (WinUSB on Windows, the udev rule on Linux).
 *
 * The backend runs the install on its own thread, so this starts it and polls
 * until it finishes. The loader screen and the dashboard banner share it.
 */

import { DriverInstallState } from '@/types';
import { call } from './rpc';

/** How often to ask whether the install has finished. */
const POLL_MS = 400;

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Runs the install and resolves with how it ended: Installed, NotNeeded or
 * Failed. If one is already running, waits for that one instead.
 */
export async function installUsbAccess(): Promise<DriverInstallState> {
  await call('startDriverInstall');
  for (;;) {
    const state = await call('getDriverInstallState');
    if (state !== DriverInstallState.Running && state !== DriverInstallState.Idle) {
      return state;
    }
    await delay(POLL_MS);
  }
}
