import { BrowserWindow, Updater, defineElectrobunRPC } from "electrobun/bun";
import { dlopen, FFIType, suffix, CString } from "bun:ffi";
import { join, sep } from "path";

const DEV_SERVER_PORT = 5173;
const DEV_SERVER_URL = `http://localhost:${DEV_SERVER_PORT}`;

// ── Cross-platform library path resolution ──────────────────
// On Linux/macOS: libmirror_backend.so / .dylib (with "lib" prefix)
// On Windows: mirror_backend.dll (no prefix)
const projectRoot = import.meta.dir.split(`${sep}build${sep}`)[0];
const isWindows = process.platform === 'win32';
const libName = isWindows
    ? `mirror_backend.${suffix}`
    : `libmirror_backend.${suffix}`;
const libPath = join(projectRoot, "mirror_backend", "target", "release", libName);
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
  get_status: { args: [], returns: FFIType.i32 },
  get_buffer_size: { args: [], returns: FFIType.i32 },
  trigger_manual_handshake: { args: [FFIType.u16, FFIType.u16], returns: FFIType.i32 },
  sync_config: { args: [FFIType.cstring], returns: FFIType.i32 },
  force_disconnect: { args: [], returns: FFIType.i32 },
  toggle_auto_reconnect: { args: [FFIType.i32], returns: FFIType.void },
  open_native_preview: { args: [FFIType.cstring], returns: FFIType.i32 },
  // OBS & system detection
  check_obs_installed: { args: [], returns: FFIType.i32 },
  check_obs_plugin_installed: { args: [], returns: FFIType.i32 },
  check_ffplay_available: { args: [FFIType.cstring], returns: FFIType.i32 },
  get_obs_plugin_dir: { args: [], returns: FFIType.ptr },
  install_obs_plugin: { args: [], returns: FFIType.i32 },
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
                const buf = Buffer.from(jsonStr + '\0');
                const result = lib.symbols.sync_config(buf as unknown as Uint8Array);
                return { success: result === 0 };
            },
            disconnectDevice: () => {
                console.log("Enterprise RPC: Disconnecting device");
                return lib.symbols.force_disconnect();
            },
            openNativePreview: async () => {
                console.log("Enterprise RPC: Opening Native Preview (ffplay)");
                const cwdBytes = new TextEncoder().encode(process.cwd() + "\0");
                lib.symbols.open_native_preview(cwdBytes);
                return 0;
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
                    driverOk
                };
            },
            // Startup checks — called once when the loader screen mounts
            getStartupChecks: () => {
                const driverOk = lib.symbols.check_driver_status() === 1;
                const cwdBytes = new TextEncoder().encode(process.cwd() + "\0");
                const ffplayOk = lib.symbols.check_ffplay_available(cwdBytes) === 1;

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
                const result = lib.symbols.install_obs_plugin();
                return { success: result === 0 };
            },
        }
    }
});

// ── Platform-specific initialization ────────────────────────
// Only call the platform-appropriate setup function
if (process.platform === 'linux') {
    lib.symbols.setup_linux_permissions();
} else if (process.platform === 'win32') {
    lib.symbols.install_windows_driver();
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
