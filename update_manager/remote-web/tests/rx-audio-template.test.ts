import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('RX audio operator controls', () => {
  it('keeps an explicit start and resume control on the operating screen', () => {
    expect(template.match(/id="rx-audio-toggle-btn"/g)).toHaveLength(1);
    expect(template).toContain('toggle.textContent = playing');
    expect(template).toContain('? "Stop Audio"');
    expect(template).toContain(': (state.audioStreaming ? "Resume Audio" : "Start Audio")');
  });

  it('primes browser audio from the Go Live gesture before starting reconnect', () => {
    const goLive = template.slice(
      template.indexOf('async function goLive()'),
      template.indexOf('function beginBridgeSession()'),
    );
    expect(goLive).toContain('await primeRxAudioContextFromGesture();');
    expect(goLive.indexOf('await primeRxAudioContextFromGesture();')).toBeLessThan(
      goLive.indexOf('beginBridgeSession();'),
    );
  });

  it('distinguishes suspended and stale browser audio from healthy playback', () => {
    expect(template).toContain('label: "Tap Audio"');
    expect(template).toContain('label: "Waiting RX"');
    expect(template).toContain('label: "Audio Stale"');
    expect(template).toContain('audioContextState=${state.audioCtx?.state || "unavailable"}');
    expect(template).toContain('audioFramesPlayed=${state.audioFramesPlayed || 0}');
  });
});
