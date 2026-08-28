import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  resolve(process.cwd(), '../templates/saturn-remote-next.html'),
  'utf8',
);

describe('operator-first control checkpoint', () => {
  it('exposes 60 metres in the primary band grid', () => {
    expect(template).toContain('data-band="5357500" type="button">60m</button>');
  });

  it('exposes authoritative A/B and split controls', () => {
    expect(template).toContain('<span class="deck-label">VFO B</span>');
    expect(template).toContain('id="vfo-b-readout"');
    expect(template).toContain('id="vfo-a-select-btn"');
    expect(template).toContain('id="vfo-b-select-btn"');
    expect(template).toContain('id="split-toggle-btn"');
    expect(template).toContain('sendTci(`split:0,${state.splitEnabled};`)');
    expect(template).toContain('const bandState = vfoBandState(txVfoFrequency());');
    expect(template).toContain('`TX ${formatFrequency(txVfoFrequency())} ${state.mode}`');
  });

  it('exposes bridge-backed attenuation, speech squelch, and radio telemetry', () => {
    expect(template).toContain('id="rx-attenuation"');
    expect(template).toContain('id="rx-att-quick-btn"');
    expect(template).toContain('id="mobile-att-btn"');
    expect(template).toContain('function cycleRxAttenuation()');
    expect(template).toContain('sendTci(`rx_attenuation:0,${state.rxAttenuationDb};`)');
    expect(template).toContain('id="rx-ssql-btn"');
    expect(template).toContain('sendTci(`rx_ssql:0,${state.rxSsqlEnabled};`)');
    expect(template).toContain('id="tx-processing-readout"');
    expect(template).toContain('id="adc-overload-annunciator"');
    expect(template).toContain('.status-chip[hidden]');
  });

  it('promotes averaging and peak hold to the panadapter toolbar', () => {
    expect(template).toContain('id="spectrum-average-toolbar-btn"');
    expect(template).toContain('id="spectrum-peak-toolbar-btn"');
    expect(template).toContain('display-spectrum-average").dispatchEvent');
    expect(template).toContain('display-spectrum-peak-hold");');
  });

  it('provides NB3, mode-aware squelch, peak tuning, DEXP, processor, and CESSB controls', () => {
    expect(template).toContain('data-nb-mode="NB3"');
    expect(template).toContain('function modeAwareSquelchKind()');
    expect(template).toContain('id="spectrum-tune-peak-btn"');
    expect(template).toContain('detectPeakInPassband(');
    expect(template).toContain('id="tx-dexp-enabled"');
    expect(template).toContain('id="tx-speech-processor-enabled"');
    expect(template).toContain('id="tx-cessb-enabled"');
    expect(template).toContain('sendTci(`tx_cessb:0,${state.txCessbEnabled};`)');
  });

  it('provides phone-size shortcuts for existing receiver DSP controls', () => {
    expect(template).toContain('id="mobile-radio-quickbar"');
    expect(template).toContain('id="mobile-nr-btn"');
    expect(template).toContain('id="mobile-nb-btn"');
    expect(template).toContain('id="mobile-anf-btn"');
    expect(template).toContain('id="mobile-wake-lock-btn"');
    expect(template).toContain('repeat(6, minmax(0, 1fr))');
    expect(template).toContain('updateWakeLockControl($("mobile-wake-lock-btn"), true)');
    expect(template).toContain('$("mobile-wake-lock-btn")?.addEventListener("click"');
    expect(template).toContain('min-height: 44px;');
  });

  it('keeps a visible desktop-layout escape in phone mode', () => {
    expect(template).toContain('button.textContent = isPhone ? "Desktop" : "Phone";');
    expect(template).toContain(':root[data-layout="phone"] .console-header #layout-btn');
    expect(template).not.toContain('.console-header #layout-btn,\n      .console-header #theme-btn { display: none; }');
  });

  it('renders a throttled audio scope from decoded RX samples', () => {
    expect(template).toContain('id="rx-audio-scope"');
    expect(template).toContain('id="instrument-rx-audio-left-fill"');
    expect(template).toContain('id="instrument-rx-audio-right-fill"');
    expect(template).toContain('function updateRxAudioMeterElements()');
    expect(template).toContain('captureRxAudioScope(playback.left, playback.right, arrivedAt)');
    expect(template).toContain('state.rxAudioScopeCapturedAt || 0) < 66');
    expect(template).toContain('_next.buildAudioScopeSnapshot(left, right, 256)');
  });
});
