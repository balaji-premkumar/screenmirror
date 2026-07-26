// The shapes the Rust backend sends across the FFI boundary.
//
// There is no code generation between Rust and TypeScript, so these are kept
// in step with `desktopApp/mirror_backend/src/` by hand. When a struct there
// gains a field, it gains one here — the `serde` name is the key.

/**
 * One entry from the backend's event log.
 *
 * `code` is the stable identifier and the only field safe to branch on.
 * `message` is an English rendering the backend produced as a fallback; the
 * UI prefers to render `code` + `params` through its own catalog so the text
 * follows the user's language. See `src/i18n/events.ts`.
 *
 * Mirrors `telemetry::log::LogEntry`.
 */
export interface LogEntry {
  timestamp: string;
  /** e.g. `usb.streaming.open_failed` */
  code: string;
  /** `INFO` | `SUCCESS` | `WARN` | `ERROR` | `FATAL` */
  level: LogLevel;
  /** Emitting subsystem, e.g. `USB`. */
  component: string;
  /** Activity within that subsystem, e.g. `handshake`. */
  action: string;
  /** Values for the `{name}` placeholders in the code's wording. */
  params: Record<string, string>;
  /** English fallback, already rendered. */
  message: string;
}

export type LogLevel = 'INFO' | 'SUCCESS' | 'WARN' | 'ERROR' | 'FATAL';

/** Mirrors `telemetry::metrics::MetricsSnapshot`. */
export interface Metrics {
  throughput_mbps: number;
  /**
   * Decode + colour conversion + sink write. Not end-to-end latency: that
   * needs a capture timestamp on the wire.
   */
  decode_latency_ms: number;
  fps_actual: number;
  frames_dropped: number;
  /** 0..1, how full the decoder's ingress queue is. */
  buffer_health: number;
}

/** 0 = stopped, 1 = starting (waiting for a keyframe), 2 = playing. */
export const PlayerState = {
  Stopped: 0,
  Starting: 1,
  Playing: 2,
} as const;

export type PlayerState = (typeof PlayerState)[keyof typeof PlayerState];

/** One poll of the backend's current state. */
export interface StatusUpdate {
  bufferSize: number;
  isActive: boolean;
  decoder: string;
  /** `type|name|vid:pid` triples. Parse with {@link parseDevice}. */
  devices: string[];
  newLogs: LogEntry[];
  metrics: Metrics;
  driverOk: boolean;
  /** Whether an OBS Studio process is running right now. */
  obsRunning: boolean;
  playerState: PlayerState;
}

/** A USB device the backend has discovered. */
export interface Device {
  /** `Accessory` once the phone has switched mode; otherwise the raw class. */
  kind: string;
  name: string;
  /** `vid:pid` in hex. */
  id: string;
  /** The original wire string, needed to call back into the RPC layer. */
  raw: string;
}

/**
 * Splits a `type|name|vid:pid` device string.
 *
 * The backend joins devices with commas and fields with pipes; doing the split
 * in one place means a component never has to know that.
 */
export function parseDevice(raw: string): Device {
  const [kind = '', name = '', id = ''] = raw.split('|');
  return { kind, name, id, raw };
}

/** Results of the one-time checks the loader screen runs. */
export interface StartupChecks {
  driverOk: boolean;
  ffplayOk: boolean;
  obsInstalled: boolean;
  obsPluginInstalled: boolean;
  obsPluginDir: string;
}

export type CheckStatus = 'pending' | 'checking' | 'ok' | 'warn' | 'error';

export interface CheckItem {
  /** i18n key, resolved at render time — not a finished string. */
  labelKey: string;
  status: CheckStatus;
  /** Already-resolved detail text, or an empty string for none. */
  detail: string;
}

/** What the stream settings panel sends to the phone. */
export interface StreamConfig {
  resolution: string;
  bitrate: string;
  fps: string;
  audioSource: string;
}
