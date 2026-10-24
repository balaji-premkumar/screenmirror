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

/** The stream's native size. The real frame size comes from the phone. */
const NOMINAL_WIDTH = 1920;
const NOMINAL_HEIGHT = 1080;

console.log(`Loading the native backend from ${libraryPath}`);

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

// WebKitGTK's DMA-BUF renderer misses repaints on Wayland, notably with
// hybrid/NVIDIA GPUs: the page scrolls and the metrics update, but the window
// does not redraw until it is resized. The shared-memory renderer does not.
// Set here as well as in package.json's scripts, for launches that bypass them.
if (process.platform === 'linux') {
  process.env.WEBKIT_DISABLE_DMABUF_RENDERER ??= '1';
}

new BrowserWindow({
  title: 'Mirror',
  url: await mainViewUrl(),
  frame: { width: 1280, height: 800, x: 200, y: 200 },
  rpc, // Without this the renderer has no transport and every call fails.
});

native.init(NOMINAL_WIDTH, NOMINAL_HEIGHT);
console.log('Backend initialised.');

// USB access is installed from the loader screen (`startDriverInstall`), once
// the window is up: it raises an elevation prompt, and doing that before any UI
// was drawn asked the person to authorise an app they could not yet see.

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
