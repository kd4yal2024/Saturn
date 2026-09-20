import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const start = template.indexOf('    function tailnetTransportActive()');
const end = template.indexOf('    function displayPacketHistoryLimit(', start);

function diagnostics(options: { mode?: string; host?: string; rtt?: number; rttAt?: number; phone?: boolean } = {}) {
  const state = {
    streamMode: options.mode ?? 'lan', layoutMode: options.phone ? 'phone' : 'desktop',
    bridgeRttMs: options.rtt ?? null, bridgeRttAt: options.rttAt ?? 1000,
    rxWorkletUnderruns: 0, bridgeOutboundQueuedBytes: 0,
  };
  const slowFrames: Array<Record<string, unknown>> = [];
  return runInNewContext(`${template.slice(start, end)};
    ({ profile: displayProfileDiagnostics, record: recordDisplaySlowFrame, slowFrames: displaySlowFrames })`, {
    state, displaySlowFrames: slowFrames,
    window: { location: { hostname: options.host ?? '192.168.0.139' }, matchMedia: () => ({ matches: false }) },
    document: { visibilityState: 'visible' },
    performance: { now: () => 2000 },
    BRIDGE_RTT_STALE_MS: 5000,
    DISPLAY_BASE_FFT_SIZE: 4096, DISPLAY_WAN_FFT_SIZE: 2048, DISPLAY_PHONE_WAN_FFT_SIZE: 1024,
    DISPLAY_BASE_RENDER_INTERVAL_MS: 33, DISPLAY_WAN_RENDER_INTERVAL_MS: 50, DISPLAY_PHONE_WAN_RENDER_INTERVAL_MS: 33,
    fftProcessor: { size: 2048 }, displaySampleRateHz: () => 384000,
    roundedTimingMs: (value: number) => value, audioLeadMs: () => 40,
  });
}

describe('display profile diagnosis', () => {
  it('identifies persisted WAN mode even on a local IP with low RTT', () => {
    const info = diagnostics({ mode: 'wan', rtt: 5 }).profile();
    expect(info.profile).toBe('wan');
    expect(info.profileReasons).toEqual(['RX Transport is set to WAN']);
    expect(info.targetFftSize).toBe(2048);
    expect(info.targetRenderIntervalMs).toBe(50);
    expect(info.binSpacingHz).toBe(187.5);
  });
  it('reports LAN defaults separately from the current FFT size', () => {
    const info = diagnostics().profile();
    expect(info.profile).toBe('lan');
    expect(info.targetFftSize).toBe(4096);
    expect(info.profileReasons).toEqual(['LAN display defaults']);
  });
  it('identifies a Tailscale hostname', () => {
    expect(diagnostics({ host: 'radio.example.ts.net' }).profile().profileReasons).toEqual(['Page uses a Tailscale host']);
  });
  it('distinguishes fresh high RTT from stale RTT', () => {
    expect(diagnostics({ rtt: 300 }).profile().profile).toBe('wan');
    expect(diagnostics({ rtt: 300, rttAt: -10000 }).profile().profile).toBe('lan');
  });
  it('uses the phone threshold and resolution for phone WAN mode', () => {
    const info = diagnostics({ phone: true, rtt: 200 }).profile();
    expect(info.profile).toBe('phone-wan');
    expect(info.targetFftSize).toBe(1024);
  });
  it('retains only 16 slow-frame contexts and ignores normal frames', () => {
    const api = diagnostics();
    api.record(33, 0.2, 1, 20);
    expect(api.slowFrames).toHaveLength(0);
    for (let i = 0; i < 20; i++) api.record(150 + i, 0.2, 1, 20);
    expect(api.slowFrames).toHaveLength(16);
    expect(api.slowFrames[0].rafGapMs).toBe(154);
    expect(api.slowFrames[15].audioQueueMs).toBe(40);
    expect(api.slowFrames[15].visibility).toBe('visible');
  });
});
