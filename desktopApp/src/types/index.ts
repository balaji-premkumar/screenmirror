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
  pipeline_latency_ms: number;
  fps_actual: number;
  frames_dropped: number;
  buffer_health: number;
}

export interface StatusUpdate {
  bufferSize: number;
  isActive: boolean;
  decoder: string;
  devices: string[];
  newLogs: LogEntry[];
  metrics: Metrics;
  driverOk: boolean;
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