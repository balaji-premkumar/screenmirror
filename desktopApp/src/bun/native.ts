/**
 * The Rust backend, loaded through `bun:ffi`.
 *
 * This module owns two things and nothing else: finding the shared library on
 * whichever platform we are on, and declaring its symbols. Everything above it
 * calls the wrappers at the bottom, which handle the C-string ownership rules
 * so no caller has to remember them.
 *
 * The symbol list must match the `#[no_mangle]` functions in
 * `mirror_backend/src/ffi/mod.rs`. There is no generated binding, so a
 * mismatch shows up as a `dlopen` failure at startup rather than a type error.
 */

import { CString, FFIType, dlopen, suffix, type Pointer } from 'bun:ffi';
import { join, sep } from 'path';

/**
 * A packaged build lives under `build/`; anything else is a dev run.
 *
 * The two layouts differ in where the native library sits, so this decides
 * both `projectRoot` and `libraryPath` below.
 */
const isDev = !import.meta.dir.includes(`${sep}build${sep}`);

/**
 * The directory the backend resolves bundled `bin/` assets against — ffplay,
 * and the prebuilt OBS plugin.
 *
 * In dev that is two levels up from `src/bun`; in a packaged build it is the
 * folder containing `bin/`.
 */
export const projectRoot = isDev
  ? join(import.meta.dir, '..', '..')
  : join(import.meta.dir, '..');

/** Windows produces `mirror_backend.dll`; Unix produces `libmirror_backend.*`. */
const libraryName =
  process.platform === 'win32' ? `mirror_backend.${suffix}` : `libmirror_backend.${suffix}`;

const libraryPath = isDev
  ? join(projectRoot, 'mirror_backend', 'target', 'release', libraryName)
  : join(projectRoot, 'bin', libraryName);

const lib = dlopen(libraryPath, {
  // Lifecycle
  init_mirror: { args: [FFIType.u32, FFIType.u32], returns: FFIType.i32 },
  stop_mirror: { args: [], returns: FFIType.i32 },

  // Drivers and permissions
  setup_linux_permissions: { args: [], returns: FFIType.i32 },
  install_windows_driver: { args: [], returns: FFIType.i32 },
  check_driver_status: { args: [], returns: FFIType.i32 },

  // Connection control
  trigger_manual_handshake: { args: [FFIType.u16, FFIType.u16], returns: FFIType.i32 },
  sync_config: { args: [FFIType.cstring], returns: FFIType.i32 },
  force_disconnect: { args: [], returns: FFIType.i32 },
  toggle_auto_reconnect: { args: [FFIType.i32], returns: FFIType.void },

  // Telemetry
  get_devices: { args: [], returns: FFIType.ptr },
  get_structured_logs: { args: [], returns: FFIType.ptr },
  get_new_logs: { args: [], returns: FFIType.ptr },
  get_metrics: { args: [], returns: FFIType.ptr },
  get_status: { args: [], returns: FFIType.i32 },
  get_buffer_size: { args: [], returns: FFIType.i32 },
  free_string: { args: [FFIType.ptr], returns: FFIType.void },

  // Playback — the only path on which this app produces sound
  start_player: { args: [FFIType.cstring], returns: FFIType.i32 },
  stop_player: { args: [], returns: FFIType.i32 },
  get_player_state: { args: [], returns: FFIType.i32 },

  // OBS
  check_obs_installed: { args: [], returns: FFIType.i32 },
  check_obs_running: { args: [], returns: FFIType.i32 },
  check_obs_plugin_installed: { args: [], returns: FFIType.i32 },
  check_ffplay_available: { args: [FFIType.cstring], returns: FFIType.i32 },
  get_obs_plugin_dir: { args: [], returns: FFIType.ptr },
  install_obs_plugin: { args: [FFIType.cstring], returns: FFIType.i32 },
  toggle_obs_feed: { args: [FFIType.i32], returns: FFIType.void },
});

const encoder = new TextEncoder();

/** Encodes a NUL-terminated string for an `FFIType.cstring` argument. */
function cstring(value: string): Uint8Array {
  return encoder.encode(`${value}\0`);
}

/**
 * Reads a string the backend allocated, and frees it.
 *
 * Every `*mut c_char` the backend returns is owned by the caller. The `finally`
 * is what makes that safe: without it, a JSON parse error anywhere downstream
 * would leak the allocation on every poll — twice a second, forever.
 */
function readOwnedString(pointer: Pointer | null): string {
  if (!pointer) return '';
  try {
    return new CString(pointer).toString();
  } finally {
    lib.symbols.free_string(pointer);
  }
}

/** Parses JSON the backend produced, falling back rather than throwing. */
function parseOwnedJson<T>(pointer: Pointer | null, fallback: T): T {
  const raw = readOwnedString(pointer);
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    console.error('Backend returned malformed JSON', error);
    return fallback;
  }
}

/**
 * The backend, as ordinary functions.
 *
 * Callers never touch pointers or `free_string`; that contract lives here.
 */
export const native = {
  init: (width: number, height: number) => lib.symbols.init_mirror(width, height),
  shutdown: () => lib.symbols.stop_mirror(),

  driverOk: () => lib.symbols.check_driver_status() === 1,
  installLinuxPermissions: () => lib.symbols.setup_linux_permissions(),
  installWindowsDriver: () => lib.symbols.install_windows_driver(),

  handshake: (vid: number, pid: number) => lib.symbols.trigger_manual_handshake(vid, pid),
  syncConfig: (config: unknown) => lib.symbols.sync_config(cstring(JSON.stringify(config))),
  disconnect: () => lib.symbols.force_disconnect(),
  setAutoReconnect: (enabled: boolean) => lib.symbols.toggle_auto_reconnect(enabled ? 1 : 0),

  isStreaming: () => lib.symbols.get_status() === 1,
  bufferSize: () => lib.symbols.get_buffer_size(),
  devices: (): string[] =>
    readOwnedString(lib.symbols.get_devices())
      .split(',')
      .filter((entry) => entry.trim().length > 0),
  newLogs: <T>(): T[] => parseOwnedJson<T[]>(lib.symbols.get_new_logs(), []),
  allLogs: <T>(): T[] => parseOwnedJson<T[]>(lib.symbols.get_structured_logs(), []),
  metrics: <T>(fallback: T): T => parseOwnedJson<T>(lib.symbols.get_metrics(), fallback),

  startPlayer: () => lib.symbols.start_player(cstring(projectRoot)),
  stopPlayer: () => lib.symbols.stop_player(),
  playerState: () => lib.symbols.get_player_state(),

  obsInstalled: () => lib.symbols.check_obs_installed() === 1,
  obsRunning: () => lib.symbols.check_obs_running() === 1,
  obsPluginInstalled: () => lib.symbols.check_obs_plugin_installed() === 1,
  obsPluginDir: () => readOwnedString(lib.symbols.get_obs_plugin_dir()),
  installObsPlugin: () => lib.symbols.install_obs_plugin(cstring(projectRoot)),
  setObsFeed: (enabled: boolean) => lib.symbols.toggle_obs_feed(enabled ? 1 : 0),

  ffplayAvailable: () => lib.symbols.check_ffplay_available(cstring(projectRoot)) === 1,
};

export { libraryPath };
