/**
 * Polls the backend and keeps a debounced view of its state.
 *
 * Two behaviours here are not incidental and should survive any rewrite:
 *
 *   * **Connection debouncing.** A single missed poll does not mean the phone
 *     is gone — the USB bus is briefly busy all the time. Reporting the
 *     disconnect immediately made the whole dashboard flicker between
 *     connected and scanning several times a minute.
 *   * **Change detection.** The status object is returned by reference when
 *     nothing meaningful moved, so React skips the re-render. Metrics are
 *     floating point and never compare equal, so they use thresholds rather
 *     than equality; without that the log panel re-rendered twice a second
 *     forever.
 */

import { useEffect, useRef, useState } from 'react';
import { PlayerState, type LogEntry, type Metrics, type StatusUpdate } from '@/types';
import { tryCall } from '@/services/rpc';

/** How often to ask the backend for its state. */
const POLL_INTERVAL_MS = 500;

/**
 * Consecutive empty polls before the UI admits the phone is gone.
 *
 * Three at 500 ms is ~1.5 s of grace, short enough that a real unplug still
 * feels immediate.
 */
const MISSED_POLLS_BEFORE_DISCONNECT = 3;

/** Log lines kept in memory. Older ones are dropped from the front. */
const MAX_LOG_LINES = 300;

const ZERO_METRICS: Metrics = {
  throughput_mbps: 0,
  decode_latency_ms: 0,
  fps_actual: 0,
  frames_dropped: 0,
  buffer_health: 0,
};

export interface DashboardStatus {
  decoder: string;
  isConnected: boolean;
  bufferSize: number;
  devices: string[];
  logs: LogEntry[];
  metrics: Metrics;
  driverOk: boolean;
  obsRunning: boolean;
  playerState: PlayerState;
}

/** Below these deltas a metrics change is not worth a re-render. */
const METRIC_EPSILON = {
  throughput_mbps: 0.05,
  fps_actual: 0.5,
  buffer_health: 0.02,
} as const;

function metricsChanged(a: Metrics, b: Metrics): boolean {
  return (
    Math.abs(a.throughput_mbps - b.throughput_mbps) > METRIC_EPSILON.throughput_mbps ||
    Math.abs(a.fps_actual - b.fps_actual) > METRIC_EPSILON.fps_actual ||
    Math.abs(a.buffer_health - b.buffer_health) > METRIC_EPSILON.buffer_health ||
    a.frames_dropped !== b.frames_dropped ||
    a.decode_latency_ms !== b.decode_latency_ms
  );
}

function devicesChanged(a: readonly string[], b: readonly string[]): boolean {
  return a.length !== b.length || a.some((device, i) => device !== b[i]);
}

const EMPTY_UPDATE: StatusUpdate = {
  bufferSize: 0,
  isActive: false,
  decoder: '',
  devices: [],
  newLogs: [],
  metrics: ZERO_METRICS,
  driverOk: false,
  obsRunning: false,
  playerState: PlayerState.Stopped,
};

export function useStatusPolling(initialDriverOk: boolean): DashboardStatus {
  const [status, setStatus] = useState<DashboardStatus>({
    decoder: '',
    isConnected: false,
    bufferSize: 0,
    devices: [],
    logs: [],
    metrics: ZERO_METRICS,
    driverOk: initialDriverOk,
    obsRunning: false,
    playerState: PlayerState.Stopped,
  });

  const missedPolls = useRef(0);

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      const data = await tryCall('getStatusUpdate', { fallback: EMPTY_UPDATE });
      if (cancelled) return;

      setStatus((prev) => {
        let isConnected = data.isActive;
        if (!data.isActive) {
          missedPolls.current += 1;
          // Hold the previous state until enough polls have missed.
          if (missedPolls.current < MISSED_POLLS_BEFORE_DISCONNECT && prev.isConnected) {
            isConnected = true;
          }
        } else {
          missedPolls.current = 0;
        }

        const nextDevices = data.devices ?? [];
        const nextMetrics = isConnected ? (data.metrics ?? prev.metrics) : ZERO_METRICS;
        const hasNewLogs = data.newLogs?.length > 0;

        const connectionChanged = prev.isConnected !== isConnected;
        const anythingMoved =
          connectionChanged ||
          hasNewLogs ||
          prev.driverOk !== data.driverOk ||
          prev.obsRunning !== data.obsRunning ||
          prev.playerState !== data.playerState ||
          prev.bufferSize !== data.bufferSize ||
          devicesChanged(prev.devices, nextDevices) ||
          metricsChanged(prev.metrics, nextMetrics);

        // Same reference: React bails out of the re-render entirely.
        if (!anythingMoved) return prev;

        return {
          ...prev,
          decoder: data.decoder || prev.decoder,
          isConnected,
          bufferSize: data.bufferSize,
          devices: devicesChanged(prev.devices, nextDevices) ? nextDevices : prev.devices,
          metrics: nextMetrics,
          driverOk: data.driverOk,
          obsRunning: data.obsRunning,
          playerState: data.playerState,
          logs: hasNewLogs
            ? [...prev.logs, ...data.newLogs].slice(-MAX_LOG_LINES)
            : // Clear the log on disconnect so the next session starts fresh.
              connectionChanged && !isConnected
              ? []
              : prev.logs,
        };
      });
    };

    const timer = setInterval(poll, POLL_INTERVAL_MS);
    void poll();

    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  return status;
}
