import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('TX setup controls', () => {
  it('keeps the operating meters in the top command cluster without Quick Step', () => {
    const commandStart = template.indexOf('class="command-strip panel"');
    const controlsStart = template.indexOf('class="control-cluster"', commandStart);
    const meterBankStart = template.indexOf('class="top-meter-bank"', controlsStart);
    const commandEnd = template.indexOf('class="audio-control-strip panel"', meterBankStart);

    expect(commandStart).toBeGreaterThanOrEqual(0);
    expect(controlsStart).toBeGreaterThan(commandStart);
    expect(meterBankStart).toBeGreaterThan(controlsStart);
    expect(meterBankStart).toBeLessThan(commandEnd);

    for (const id of [
      'smeter-svg',
      'meter-readout',
      'txpwr-svg',
      'tx-power-readout',
      'swrmeter-svg',
      'swr-readout',
    ]) {
      const position = template.indexOf(`id="${id}"`);
      expect(position).toBeGreaterThan(meterBankStart);
      expect(position).toBeLessThan(commandEnd);
      expect(template.match(new RegExp(`id="${id}"`, 'g'))).toHaveLength(1);
    }

    const auxMeters = template.indexOf('class="aux-meter-row"', meterBankStart);
    const telemetry = template.indexOf('class="top-meter-telemetry"', auxMeters);
    expect(auxMeters).toBeGreaterThan(meterBankStart);
    expect(telemetry).toBeGreaterThan(auxMeters);
    expect(telemetry).toBeLessThan(commandEnd);
    expect(template).toContain('class="top-meter-telemetry-title">Telemetry');
    expect(template).toContain('class="top-meter-telemetry-meta">Observed radio state');
    expect(template).not.toContain('data-phone-panel="telemetry"');
    expect(template).not.toContain('class="direct-frequency-control"');
    expect(template).not.toContain('id="vfo-a-input"');
    expect(template).not.toContain('id="freq-entry-open-btn"');
    expect(template).not.toContain('id="vfo-step-down-btn"');
    expect(template).not.toContain('id="vfo-step-up-btn"');
  });

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
