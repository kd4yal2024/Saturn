import { describe, expect, it } from 'vitest';
import { createAppState, PHONE_PANEL_DEFAULTS } from '../src/state/app-state';

describe('PHONE_PANEL_DEFAULTS', () => {
  it('has expected defaults', () => {
    expect(PHONE_PANEL_DEFAULTS.session).toBe(false);
    expect(PHONE_PANEL_DEFAULTS.routing).toBe(true);
    expect(PHONE_PANEL_DEFAULTS.tuning).toBe(false);
    expect(PHONE_PANEL_DEFAULTS.demod).toBe(false);
    expect(PHONE_PANEL_DEFAULTS.display).toBe(false);
    expect(PHONE_PANEL_DEFAULTS.log).toBe(true);
    expect(PHONE_PANEL_DEFAULTS.telemetry).toBe(true);
    expect(PHONE_PANEL_DEFAULTS.tx).toBe(false);
  });

  it('is frozen', () => {
    expect(Object.isFrozen(PHONE_PANEL_DEFAULTS)).toBe(true);
  });
});

describe('createAppState', () => {
  it('returns a fresh object each call', () => {
    const a = createAppState();
    const b = createAppState();
    expect(a).not.toBe(b);
  });

  // ── Connection defaults ──
  it('ws defaults to null', () => expect(createAppState().ws).toBeNull());
  it('connected defaults to false', () => expect(createAppState().connected).toBe(false));
  it('demoMode defaults to false', () => expect(createAppState().demoMode).toBe(false));

  // ── Radio defaults ──
  it('mode defaults to USB', () => expect(createAppState().mode).toBe('USB'));
  it('vfoA defaults to 14200000', () => expect(createAppState().vfoA).toBe(14200000));
  it('sampleRate defaults to 192000', () => expect(createAppState().sampleRate).toBe(192000));
  it('streamMode defaults to lan', () => expect(createAppState().streamMode).toBe('lan'));

  // ── RX DSP defaults ──
  it('rxVolumeDb defaults to -10', () => expect(createAppState().rxVolumeDb).toBe(-10));
  it('agcMode defaults to MEDIUM', () => expect(createAppState().agcMode).toBe('MEDIUM'));
  it('agcGain defaults to 80', () => expect(createAppState().agcGain).toBe(80));

  // ── Filter defaults ──
  it('filterLow defaults to 50', () => expect(createAppState().filterLow).toBe(50));
  it('filterHigh defaults to 3050', () => expect(createAppState().filterHigh).toBe(3050));

  // ── TX defaults ──
  it('txEnabled defaults to false', () => expect(createAppState().txEnabled).toBe(false));
  it('txDrive defaults to a conservative 10 W target', () => expect(createAppState().txDrive).toBe(10));

  // ── EQ/CFC defaults ──
  it('rxEqBands has 11 zeros', () => {
    const bands = createAppState().rxEqBands;
    expect(bands).toHaveLength(11);
    expect(bands.every(v => v === 0)).toBe(true);
  });

  // ── Display defaults ──
  it('displayZoom defaults to 1', () => expect(createAppState().displayZoom).toBe(1));
  it('spectrumAutoRange defaults to true', () => expect(createAppState().spectrumAutoRange).toBe(true));
  it('waterfallPalette defaults to classic', () => expect(createAppState().waterfallPalette).toBe('classic'));

  // ── FFT defaults ──
  it('latestBins is Float32Array of 1024', () => {
    const bins = createAppState().latestBins;
    expect(bins).toBeInstanceOf(Float32Array);
    expect(bins.length).toBe(1024);
  });

  // ── Audio defaults ──
  it('audioSampleRate defaults to 48000', () => expect(createAppState().audioSampleRate).toBe(48000));
  it('audioCtx defaults to null', () => expect(createAppState().audioCtx).toBeNull());

  // ── UI defaults ──
  it('theme defaults to dark', () => expect(createAppState().theme).toBe('dark'));
  it('layoutMode defaults to desktop', () => expect(createAppState().layoutMode).toBe('desktop'));
  it('phonePanels is a copy of defaults', () => {
    const s = createAppState();
    expect(s.phonePanels).toEqual({ ...PHONE_PANEL_DEFAULTS });
    expect(s.phonePanels).not.toBe(PHONE_PANEL_DEFAULTS);
  });

  // ── Performance defaults ──
  it('perfSnapshotsMax defaults to 600', () => expect(createAppState().perfSnapshotsMax).toBe(600));
});
