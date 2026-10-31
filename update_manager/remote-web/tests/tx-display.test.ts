import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { acceptsTxDisplayFrame, latestTxDisplayWindow, txDisplayStatus } from '../src/dsp/tx-display';
import { FftProcessor } from '../src/dsp/fft';
import { visibleBinsForDisplay } from '../src/dsp/display';

const html = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
describe('live TX baseband display', () => {
  it('requires active connected TX, bounded whole complex samples and the real rate', () => {
    expect(acceptsTxDisplayFrame(true, true, 192000, 7680, 30784)).toBe(true);
    for (const [connected, active, rate, floats, bytes] of [
      [false, true, 192000, 7680, 30784], [true, false, 192000, 7680, 30784],
      [true, true, 384000, 7680, 30784], [true, true, 192000, 7681, 30788],
      [true, true, 192000, 16384, 65600], [true, true, 192000, 7680, 100],
    ] as const) expect(acceptsTxDisplayFrame(connected, active, rate, floats, bytes)).toBe(false);
  });
  it('does not splice discontiguous snapshots or retain an older generation window', () => {
    const old = new Float32Array(7680).fill(.5), fresh = new Float32Array(7680).fill(-.5);
    expect(latestTxDisplayWindow([old, fresh])).toBe(fresh);
    expect(latestTxDisplayWindow([])).toBeNull();
    expect(html).toContain('if (sourceKind === "tx") state.iqPackets = [];');
    expect(html).toContain('if (state.displayIqSource === "tx") resetDisplayHistory();');
  });
  it('marks missing/stale/paused/disconnected input without claiming RF evidence', () => {
    expect(txDisplayStatus(true, true, false, 100, 200)).toContain('Live TX baseband');
    expect(txDisplayStatus(true, true, false, 100, 200)).toContain('not RF feedback');
    expect(txDisplayStatus(true, true, false, null, 200)).toContain('Waiting');
    expect(txDisplayStatus(true, true, false, 100, 900)).toContain('stale');
    expect(txDisplayStatus(true, true, true, 100, 200)).toContain('not live');
    expect(txDisplayStatus(false, true, false, 100, 200)).toContain('disconnected');
  });
  it.each([1, -1])('shows both native-convention tones on the expected side (%s)', sign => {
    const iq = new Float32Array(7680);
    for (let n = 0; n < iq.length / 2; n++) {
      const phase = 2 * Math.PI * n / 192000;
      iq[n * 2] = .2 * (Math.cos(700 * phase) + Math.cos(1900 * phase));
      iq[n * 2 + 1] = sign * .2 * (Math.sin(700 * phase) + Math.sin(1900 * phase));
    }
    const bins = new FftProcessor(2048).transform(iq.subarray(iq.length - 4096));
    const visible = visibleBinsForDisplay(bins, 4);
    // Display deliberately reverses digital bins; positive native baseband
    // corresponds to LSB on this established radio path.
    for (const hz of [700, 1900]) {
      const index = Math.round(visible!.length * (.5 - sign * hz / 48000));
      const peak = Math.max(...visible!.slice(index - 2, index + 3));
      expect(peak).toBeGreaterThan(-30);
    }
  });
  it('puts a shared two-tone action at the top of PS, retaining Setup controls', () => {
    expect(html.match(/id="ps-two-tone-btn"/g)).toHaveLength(1);
    expect(html.match(/id="two-tone-btn"/g)).toHaveLength(1);
    const panel = html.slice(html.indexOf('id="ps-control-panel"'), html.indexOf('<div class="stack right-rail">'));
    expect(panel.indexOf('id="ps-two-tone-btn"')).toBeLessThan(panel.indexOf('id="ps-control-body"'));
    const handler = html.slice(html.indexOf('$("ps-two-tone-btn").addEventListener'), html.indexOf('// ── CFC Band'));
    expect(handler).toContain('setTwoToneEnabled(!state.twoToneEnabled)');
    expect(handler).not.toMatch(/trx:|tx_puresignal:|saturn_ps_measurement:/);
  });
});
