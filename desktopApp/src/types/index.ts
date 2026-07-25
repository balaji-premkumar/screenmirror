// Types for the Mirror Core Enterprise application

export interface LogEntry {
  timestamp: string;
  level: string;
  module: string;
  thread: string;
  message: string;
}

export interface Metrics {
  throughput_mbps: number;
  /** Decode + colour conversion + sink write. Not end-to-end latency: that
   *  needs a capture timestamp on the wire (ISSUES.md item 1). */
  decode_latency_ms: number;
  fps_actual: number;
  frames_dropped: number;
  buffer_health: number;
}

/** 0 = stopped, 1 = starting (waiting for a keyframe), 2 = playing */
export type PlayerState = 0 | 1 | 2;

export interface StatusUpdate {
  bufferSize: number;
  isActive: boolean;
  decoder: string;
  devices: string[];
  newLogs: LogEntry[];
  metrics: Metrics;
  driverOk: boolean;
  /** OBS Studio process detected right now */
  obsRunning: boolean;
  playerState: PlayerState;
}

export interface StartupChecks {
  driverOk: boolean;
  ffplayOk: boolean;
  obsInstalled: boolean;
  obsPluginInstalled: boolean;
  obsPluginDir: string;
}

export type CheckStatus = 'pending' | 'checking' | 'ok' | 'warn' | 'error';

export interface CheckItem {
  label: string;
  status: CheckStatus;
  detail: string;
}

declare global {
  interface Window {
    __mirrorRpc: {
      request: (method: string, data?: any) => Promise<any>;
    };
    Electrobun: {
      rpc: {
        request: (method: string, data?: any) => Promise<any>;
      }
    }
  }
}