/**
 * What the dashboard should say right now.
 *
 * The backend reports facts (devices seen, link up, frames flowing); a person
 * wants to know what to do next. Keeping the translation between the two in
 * plain functions means the wording logic can be tested without a phone, and
 * the components stay about layout.
 */

import { parseDevice, type Metrics } from '@/types';

/** The device kind the phone reports once it has entered accessory mode. */
export const ACCESSORY = 'Accessory';

export type StageId =
  /** No phone on the bus. */
  | 'no-phone'
  /** A phone is plugged in but has not been switched into accessory mode. */
  | 'phone-found'
  /** The cable link is up; nothing is being captured yet. */
  | 'ready'
  /** Start was pressed; waiting for the phone's screen-sharing prompt. */
  | 'waiting-for-phone'
  /** Frames are arriving. */
  | 'mirroring';

export interface StageInput {
  devices: readonly string[];
  isConnected: boolean;
  fps: number;
  /** The user pressed Start and has not yet seen frames or pressed Stop. */
  startRequested: boolean;
}

export function deriveStage({ devices, isConnected, fps, startRequested }: StageInput): StageId {
  if (isConnected) {
    if (fps > 0) return 'mirroring';
    return startRequested ? 'waiting-for-phone' : 'ready';
  }
  return devices.length > 0 ? 'phone-found' : 'no-phone';
}

/** The device to talk about in the headline: the connected one, else the first. */
export function primaryDevice(devices: readonly string[]) {
  const parsed = devices.map(parseDevice);
  return parsed.find((d) => d.kind === ACCESSORY) ?? parsed[0] ?? null;
}

export type HealthLevel = 'good' | 'warn' | 'bad';

export interface Health {
  level: HealthLevel;
  /** i18n key for the one-line verdict. */
  verdictKey: string;
  /** i18n key for the suggestion, when there is something to try. */
  hintKey?: string;
}

/** Buffer fill above this is healthy; below the lower bound it is failing. */
const BUFFER_HEALTHY = 0.6;
const BUFFER_DEGRADED = 0.3;
/** Frame rate below this fraction of the target reads as stutter. */
const FPS_SHORTFALL = 0.85;
/** Extra delay beyond this reads as lag. */
const DELAY_NOTICEABLE_MS = 120;

/**
 * Turns the raw numbers into a verdict and, for the bad cases, the single
 * most useful thing to try. Worst problem wins: a person can act on one hint,
 * not four.
 */
export function assessStream(metrics: Metrics, targetFps: number): Health {
  if (metrics.buffer_health < BUFFER_DEGRADED) {
    return { level: 'bad', verdictKey: 'health.verdict.dropping', hintKey: 'health.hint.lowerQuality' };
  }
  if (metrics.link_delay_ms > DELAY_NOTICEABLE_MS) {
    return { level: 'warn', verdictKey: 'health.verdict.lagging', hintKey: 'health.hint.lowerBitrate' };
  }
  if (metrics.fps_actual < targetFps * FPS_SHORTFALL || metrics.buffer_health < BUFFER_HEALTHY) {
    return { level: 'warn', verdictKey: 'health.verdict.uneven', hintKey: 'health.hint.lowerFrameRate' };
  }
  return { level: 'good', verdictKey: 'health.verdict.smooth' };
}

export interface Preset {
  id: 'light' | 'balanced' | 'sharp' | 'maximum';
  resolution: string;
  bitrate: string;
  fps: string;
}

/** Values are sent verbatim to the phone; see the note in the quality panel. */
export const PRESETS: readonly Preset[] = [
  { id: 'light', resolution: '720p', bitrate: '8 Mbps', fps: '30' },
  { id: 'balanced', resolution: '1080p', bitrate: '12 Mbps', fps: '60' },
  { id: 'sharp', resolution: '2K', bitrate: '20 Mbps', fps: '60' },
  { id: 'maximum', resolution: '4K', bitrate: '50 Mbps', fps: '60' },
];

export function matchPreset(config: { resolution: string; bitrate: string; fps: string }) {
  return PRESETS.find(
    (p) => p.resolution === config.resolution && p.bitrate === config.bitrate && p.fps === config.fps,
  );
}
