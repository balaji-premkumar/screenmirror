/**
 * Entry point for the Bun process: opens the window, starts the backend, and
 * makes sure it is torn down again.
 *
 * The FFI bindings live in `native.ts` and the RPC surface in `rpc.ts`. This
 * file is the startup order and nothing else, because the startup order is the
 * part with the constraints.
 */

import { BrowserWindow, Updater } from 'electrobun/bun';
import { libraryPath, native } from './native';
import { rpc } from './rpc';

const DEV_SERVER_URL = 'http://localhost:5173';

/**
 * Delay before offering to install USB permissions.
 *
 * Both the Linux and Windows paths raise a system elevation prompt. Running
 * that at module load meant the prompt appeared before any UI had been drawn,
 * so the user was asked to authorise an elevated action by an application they
 * could not yet see — and startup blocked on their answer.
 */
const DRIVER_PROMPT_DELAY_MS = 1500;

/** The stream's native size. The real frame size comes from the phone. */
const NOMINAL_WIDTH = 1920;
const NOMINAL_HEIGHT = 1080;

console.log(`Loading the native backend from ${libraryPath}`);

/**
 * Installs USB permissions, but only when they are actually missing.
 *
 * Re-running it on every launch would raise an elevation prompt each time for
 * nothing.
 */
function ensureUsbAccess() {
  if (native.driverOk()) {
    console.log('USB access is already configured.');
    return;
  }

  if (process.platform === 'linux') {
    console.log('Installing the udev rule for accessory access…');
    console.log(`udev rule installation returned ${native.installLinuxPermissions()}`);
  } else if (process.platform === 'win32') {
    console.log('Installing the WinUSB driver for the accessory…');
    console.log(`WinUSB installation returned ${native.installWindowsDriver()}`);
  }
}

async function mainViewUrl(): Promise<string> {
  const channel = await Updater.localInfo.channel();
  if (channel === 'dev') {
    try {
      await fetch(DEV_SERVER_URL, { method: 'HEAD' });
      return DEV_SERVER_URL;
    } catch {
      // No dev server running — fall through to the bundled view.
    }
  }
  return 'views://mainview/index.html';
}

new BrowserWindow({
  title: 'Mirror',
  url: await mainViewUrl(),
  frame: { width: 1280, height: 800, x: 200, y: 200 },
  rpc, // Without this the renderer has no transport and every call fails.
});

native.init(NOMINAL_WIDTH, NOMINAL_HEIGHT);
console.log('Backend initialised.');

setTimeout(ensureUsbAccess, DRIVER_PROMPT_DELAY_MS);

/**
 * Tears the native pipeline down so the USB interface is released and the
 * shared-memory segments are unlinked instead of leaking to the next run.
 */
let shutdownDone = false;
function shutdown() {
  if (shutdownDone) return;
  shutdownDone = true;
  try {
    native.shutdown();
  } catch (error) {
    console.error('Shutting the backend down failed', error);
  }
}

process.on('exit', shutdown);
process.on('SIGINT', () => {
  shutdown();
  process.exit(0);
});
process.on('SIGTERM', () => {
  shutdown();
  process.exit(0);
});
