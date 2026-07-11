import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('TX setup controls', () => {
  it('places the complete two-tone test inside the TX setup panel', () => {
    const txSetupStart = template.indexOf('id="setup-panel-tx"');
    const twoToneSection = template.indexOf('id="setup-tx-two-tone-section"');
    const txSetupEnd = template.indexOf('class="audio-control-strip panel"');

    expect(txSetupStart).toBeGreaterThanOrEqual(0);
    expect(twoToneSection).toBeGreaterThan(txSetupStart);
    expect(twoToneSection).toBeLessThan(txSetupEnd);

    for (const id of [
      'two-tone-btn',
      'two-tone-freq1',
      'two-tone-freq2',
      'two-tone-level',
      'two-tone-delay',
      'two-tone-invert-lsb',
    ]) {
      expect(template.match(new RegExp(`id="${id}"`, 'g'))).toHaveLength(1);
    }
  });

  it('places PureSignal controls and status inside the TX setup panel', () => {
    const txSetupStart = template.indexOf('id="setup-panel-tx"');
    const pureSignalSection = template.indexOf('id="setup-tx-puresignal-section"');
    const txSetupEnd = template.indexOf('class="audio-control-strip panel"');

    expect(pureSignalSection).toBeGreaterThan(txSetupStart);
    expect(pureSignalSection).toBeLessThan(txSetupEnd);
    for (const id of [
      'setup-tx-puresignal-enabled',
      'setup-tx-puresignal-auto-attenuate',
      'setup-tx-puresignal-attenuation',
      'setup-tx-puresignal-reset',
      'setup-tx-puresignal-state',
      'setup-tx-puresignal-feedback',
      'setup-tx-puresignal-calibrations',
      'setup-tx-puresignal-correcting',
    ]) {
      expect(template.match(new RegExp(`id="${id}"`, 'g'))).toHaveLength(1);
    }
  });

  it('adds rate-limited PureSignal telemetry to the Operator Log export', () => {
    expect(template).toContain('function syncPureSignalOperatorTelemetry(previous, next)');
    expect(template).toContain('performance.now() - pureSignalOperatorTelemetryLastAt >= 5000');
    expect(template).toContain('pureSignalFeedbackLevel=${');
    expect(template).toContain('pureSignalFeedbackPackets=${');
    expect(template).toContain('pureSignalFeedbackGaps=${');
    expect(template).toContain('recordOperatorFault("PureSignal feedback fault"');
    expect(template).toContain('recordOperatorFault("PureSignal feedback gaps"');
  });
});
