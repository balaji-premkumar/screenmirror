import { describe, expect, test } from 'bun:test';
import type { Metrics } from '@/types';
import { assessStream, deriveStage, matchPreset, primaryDevice } from './stage';

const base = { devices: [] as string[], isConnected: false, fps: 0, startRequested: false };

describe('deriveStage', () => {
  test('walks the plug-in, connect, start, mirror sequence', () => {
    expect(deriveStage(base)).toBe('no-phone');
    expect(deriveStage({ ...base, devices: ['Phone|Pixel|18d1:4ee1'] })).toBe('phone-found');
    expect(deriveStage({ ...base, isConnected: true })).toBe('ready');
    expect(deriveStage({ ...base, isConnected: true, startRequested: true })).toBe('waiting-for-phone');
    expect(deriveStage({ ...base, isConnected: true, startRequested: true, fps: 59 })).toBe('mirroring');
  });

  test('frames arriving win over a stale start flag', () => {
    expect(deriveStage({ ...base, isConnected: true, fps: 30 })).toBe('mirroring');
  });

  test('a dropped link goes back to finding the phone, whatever Start said', () => {
    expect(deriveStage({ ...base, startRequested: true, fps: 60 })).toBe('no-phone');
  });
});

describe('primaryDevice', () => {
  test('prefers the device already in accessory mode', () => {
    const d = primaryDevice(['Phone|Old|1:1', 'Accessory|Pixel 8|2:2']);
    expect(d?.name).toBe('Pixel 8');
  });
  test('is null with nothing plugged in', () => {
    expect(primaryDevice([])).toBeNull();
  });
});

const healthy: Metrics = {
  throughput_mbps: 12,
  decode_latency_ms: 4,
  link_delay_ms: 10,
  fps_actual: 60,
  frames_dropped: 0,
  buffer_health: 0.95,
};

describe('assessStream', () => {
  test('smooth when frames arrive on time', () => {
    expect(assessStream(healthy, 60).level).toBe('good');
  });
  test('a full queue is the worst case and suggests lower quality', () => {
    const h = assessStream({ ...healthy, buffer_health: 0.1, link_delay_ms: 500 }, 60);
    expect(h).toMatchObject({ level: 'bad', hintKey: 'health.hint.lowerQuality' });
  });
  test('growing delay is reported as lag before it becomes drops', () => {
    expect(assessStream({ ...healthy, link_delay_ms: 300 }, 60)).toMatchObject({
      level: 'warn',
      verdictKey: 'health.verdict.lagging',
    });
  });
  test('a frame rate well under target is uneven', () => {
    expect(assessStream({ ...healthy, fps_actual: 40 }, 60).verdictKey).toBe('health.verdict.uneven');
  });
});

describe('matchPreset', () => {
  test('finds a preset only when all three values agree', () => {
    expect(matchPreset({ resolution: '1080p', bitrate: '12 Mbps', fps: '60' })?.id).toBe('balanced');
    expect(matchPreset({ resolution: '1080p', bitrate: '12 Mbps', fps: '90' })).toBeUndefined();
  });
});
