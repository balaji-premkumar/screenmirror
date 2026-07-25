import { BrowserWindow, Updater, defineElectrobunRPC } from "electrobun/bun";
import { dlopen, FFIType, suffix, CString } from "bun:ffi";
import { join, sep } from "path";

const DEV_SERVER_PORT = 5173;
const DEV_SERVER_URL = `http://localhost:${DEV_SERVER_PORT}`;

// ── Cross-platform library path resolution ──────────────────
// On Linux/macOS: libmirror_backend.so / .dylib (with "lib" prefix)
// On Windows: mirror_backend.dll (no prefix)
const isDev = !import.meta.dir.includes(`${sep}build${sep}`);
// In dev, projectRoot is two levels up from src/bun. In prod, it's the folder containing the bin/ folder.
const projectRoot = isDev ? join(import.meta.dir, "..", "..") : join(import.meta.dir, "..");
console.log("Enterprise: Project Root identified as:", projectRoot);
const isWindows = process.platform === 'win32';
const libName = isWindows
    ? `mirror_backend.${suffix}`
    : `libmirror_backend.${suffix}`;
const libPath = isDev 
    ? join(projectRoot, "mirror_backend", "target", "release", libName)
    : join(projectRoot, "bin", libName); // Built app usually puts libs in bin/ or adjacent
console.log("Enterprise: Loading Rust Library from:", libPath);

const lib = dlopen(libPath, {
  setup_linux_permissions: { args: [], returns: FFIType.i32 },
  install_windows_driver: { args: [], returns: FFIType.i32 },
  check_driver_status: { args: [], returns: FFIType.i32 },
  get_devices: { args: [], returns: FFIType.ptr },
  get_structured_logs: { args: [], returns: FFIType.ptr },
  get_new_logs: { args: [], returns: FFIType.ptr },
  get_metrics: { args: [], returns: FFIType.ptr },
  free_string: { args: [FFIType.ptr], returns: FFIType.void },
  init_mirror: { args: [FFIType.u32, FFIType.u32], returns: FFIType.i32 },
  stop_mirror: { args: [], returns: FFIType.i32 },
  get_status: { args: [], returns: FFIType.i32 },
  get_buffer_size: { args: [], returns: FFIType.i32 },
  trigger_manual_handshake: { args: [FFIType.u16, FFIType.u16], returns: FFIType.i32 },
  sync_config: { args: [FFIType.cstring], returns: FFIType.i32 },
  force_disconnect: { args: [], returns: FFIType.i32 },
  toggle_auto_reconnect: { args: [FFIType.i32], returns: FFIType.void },
  // Player (ffplay) — the only path that produces sound on the desktop
  start_player: { args: [FFIType.cstring], returns: FFIType.i32 },
  stop_player: { args: [], returns: FFIType.i32 },
  get_player_state: { args: [], returns: FFIType.i32 },
  // OBS & system detection
  check_obs_installed: { args: [], returns: FFIType.i32 },
  check_obs_running: { args: [], returns: FFIType.i32 },
  check_obs_plugin_installed: { args: [], returns: FFIType.i32 },
  check_ffplay_available: { args: [FFIType.cstring], returns: FFIType.i32 },
  get_obs_plugin_dir: { args: [], returns: FFIType.ptr },
  install_obs_plugin: { args: [FFIType.cstring], returns: FFIType.i32 },
  toggle_obs_feed: { args: [FFIType.i32], returns: FFIType.void },
});

// Setup Electrobun RPC 2.0
const rpc = defineElectrobunRPC('bun', {
    handlers: {
        requests: {
            repairDrivers: () => {
                console.log("Enterprise RPC: Repairing Drivers...");
                if (process.platform === 'linux') {
                    return lib.symbols.setup_linux_permissions();
                } else if (process.platform === 'win32') {
                    return lib.symbols.install_windows_driver();
                }
                return 0;
            },
            triggerHandshake: (params?: unknown) => {
                const data = params as { vid: number, pid: number };
                console.log(`Enterprise RPC: Handshake for ${data.vid.toString(16)}:${data.pid.toString(16)}`);
                return lib.symbols.trigger_manual_handshake(data.vid, data.pid);
            },
            syncConfig: (config: any) => {
                const jsonStr = JSON.stringify(config);
                console.log("Enterprise RPC: Syncing configuration to Companion:", jsonStr);
                const buf = new TextEncoder().encode(jsonStr + "\0");
                const result = lib.symbols.sync_config(buf);
                return { success: result === 0 };
            },
            disconnectDevice: () => {
                console.log("Enterprise RPC: Disconnecting device");
                return lib.symbols.force_disconnect();
            },
            // NOTE: no stopMirror endpoint. `stop_mirror()` tears down the USB
            // listener, decoder and shared memory, and only process exit calls
            // it. Exposing it as an RPC was a trap: nothing in the UI re-runs
            // init_mirror(), so a "stop" button wired to it would have left the
            // app inert until restart. Stopping a stream is `syncConfig`
            // with command "stop", which is what the UI actually uses.
            // Playback: hands video AND audio to a child ffplay process.
            // The backend itself never opens an audio device.
            startPlayer: () => {
                console.log("Enterprise RPC: Starting ffplay playback (video + audio)");
                const rootBytes = new TextEncoder().encode(projectRoot + "\0");
                const result = lib.symbols.start_player(rootBytes);
                return { success: result === 0 };
            },
            stopPlayer: () => {
                console.log("Enterprise RPC: Stopping ffplay playback");
                lib.symbols.stop_player();
                return { success: true };
            },
            toggleObsFeed: (data: any) => {
                console.log(`Enterprise RPC: OBS Feed toggled to ${data.enabled}`);
                lib.symbols.toggle_obs_feed(data.enabled ? 1 : 0);
                return { success: true };
            },
            toggleAutoReconnect: (data: any) => {
                console.log(`Enterprise RPC: Auto-reconnect toggled to ${data.enabled}`);
                lib.symbols.toggle_auto_reconnect(data.enabled ? 1 : 0);
                return { success: true };
            },
            // Polling endpoint — View requests telemetry from Bun
            getStatusUpdate: () => {
                const devString = readCString(lib.symbols.get_devices());
                const devices = devString ? devString.split(',').filter(s => s.trim().length > 0) : [];
                const newLogsJson = readCString(lib.symbols.get_new_logs());
                let newLogs: any[] = [];
                try { newLogs = JSON.parse(newLogsJson); } catch (e) { }
                const metricsJson = readCString(lib.symbols.get_metrics());
                let metrics = {};
                try { metrics = JSON.parse(metricsJson); } catch (e) { }
                const driverOk = lib.symbols.check_driver_status() === 1;

                return {
                    bufferSize: lib.symbols.get_buffer_size(),
                    isActive: lib.symbols.get_status() === 1,
                    decoder: "Enterprise HW Decoder",
                    devices,
                    newLogs,
                    metrics,
                    driverOk,
                    // Cached for 3s inside Rust — safe to ask on every tick.
                    obsRunning: lib.symbols.check_obs_running() === 1,
                    // 0 = stopped, 1 = starting, 2 = playing
                    playerState: lib.symbols.get_player_state(),
                };
            },
            // Startup checks — called once when the loader screen mounts
            getStartupChecks: () => {
                const driverOk = lib.symbols.check_driver_status() === 1;
                const rootBytes = new TextEncoder().encode(projectRoot + "\0");
                const ffplayOk = lib.symbols.check_ffplay_available(rootBytes) === 1;

                return {
                    driverOk,
                    ffplayOk,
                    obsInstalled: lib.symbols.check_obs_installed() === 1,
                    obsPluginInstalled: lib.symbols.check_obs_plugin_installed() === 1,
                    obsPluginDir: readCString(lib.symbols.get_obs_plugin_dir()),
                };
            },
            installObsPlugin: () => {
                console.log("Enterprise RPC: Installing OBS plugin...");
                const rootBytes = new TextEncoder().encode(projectRoot + "\0");
                const result = lib.symbols.install_obs_plugin(rootBytes);
                return { success: result === 0 };
            },
        }
    }
});

// ── Platform-specific driver automation ─────────────────────
// Run once at startup, and only when the driver is actually missing — both
// paths raise an elevation prompt (pkexec / UAC), so re-running them on every
// launch would nag the user for nothing.
function ensureUsbDriver() {
    if (lib.symbols.check_driver_status() === 1) {
        console.log("USB driver/permissions already configured.");
        return;
    }

    if (process.platform === 'linux') {
        console.log("Installing udev rules for AOA access...");
        const result = lib.symbols.setup_linux_permissions();
        console.log(`udev rule installation returned ${result}`);
    } else if (process.platform === 'win32') {
        console.log("Installing WinUSB driver for AOA accessory...");
        const result = lib.symbols.install_windows_driver();
        console.log(`WinUSB driver installation returned ${result}`);
    }
}

async function getMainViewUrl(): Promise<string> {
	const channel = await Updater.localInfo.channel();
	if (channel === "dev") {
		try {
			await fetch(DEV_SERVER_URL, { method: "HEAD" });
			return DEV_SERVER_URL;
		} catch { }
	}
	return "views://mainview/index.html";
}

const url = await getMainViewUrl();
// eslint-disable-next-line @typescript-eslint/no-unused-vars
new BrowserWindow({
	title: "Mirroring Receiver - Enterprise",
	url,
	frame: { width: 1280, height: 800, x: 200, y: 200 },
    rpc // CRITICAL: Link RPC to window transport
});

function readCString(ptr: any): string {
    if (!ptr) return "";
    try {
        return new CString(ptr).toString();
    } finally {
        lib.symbols.free_string(ptr);
    }
}

lib.symbols.init_mirror(1920, 1080);
console.log("Enterprise Mirroring Receiver initialized.");

// Deferred until the window exists. Running this at module load meant the
// pkexec/UAC prompt appeared before any UI had been drawn, so the user was
// asked to authorise an elevated action by an application they could not yet
// see. It also blocked startup on their response.
setTimeout(ensureUsbDriver, 1500);

// Tear the native pipeline down cleanly so USB interfaces are released and
// the shared memory segments are unlinked instead of leaking to the next run.
let shutdownDone = false;
function shutdown() {
    if (shutdownDone) return;
    shutdownDone = true;
    try {
        lib.symbols.stop_mirror();
    } catch (e) {
        console.error("stop_mirror failed during shutdown", e);
    }
}

process.on("exit", shutdown);
process.on("SIGINT", () => { shutdown(); process.exit(0); });
process.on("SIGTERM", () => { shutdown(); process.exit(0); });
