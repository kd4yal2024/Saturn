import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it, vi } from 'vitest';
import { createAppState } from '../src/state/app-state';
import { applyDisplayPrefsToState } from '../src/state/apply-prefs';
import { displayPrefsFromState } from '../src/state/prefs-from-state';
import { normalizeDisplayPrefs } from '../src/settings/normalize';

const template = readFileSync('../templates/saturn-remote-next.html', 'utf8');
const helper = template.slice(template.indexOf('    function transceiverClarityAppearance()'),
  template.indexOf('    function currentDisplayPrefs()'));
const binding = template.slice(template.indexOf('      let previousDisplayAppearance = null;'),
  template.indexOf('      $("display-spectrum-trace-color").addEventListener'));

describe('transceiver clarity appearance', () => {
  it('persists Enhanced spectrum colors and restores solid colors without changing RF settings', () => {
    const state = createAppState();
    expect(state.spectrumEnhancedColors).toBe(false);
    const sampleRate = state.sampleRate;
    applyDisplayPrefsToState({ spectrumEnhancedColors: true }, state);
    const saved = JSON.parse(JSON.stringify(displayPrefsFromState(state)));
    const restored = createAppState();
    applyDisplayPrefsToState(normalizeDisplayPrefs(saved), restored);
    expect(restored.spectrumEnhancedColors).toBe(true);
    expect(restored.sampleRate).toBe(sampleRate);
    applyDisplayPrefsToState({ spectrumEnhancedColors: false }, restored);
    expect(restored.spectrumEnhancedColors).toBe(false);
    expect(normalizeDisplayPrefs({}).spectrumEnhancedColors).toBe(false);
    expect(normalizeDisplayPrefs(JSON.parse('{"spectrumEnhancedColors":"false"}')).spectrumEnhancedColors).toBe(false);
  });
  it('applies and undoes only appearance, including repeated clicks', () => {
    const appearance = runInNewContext(`${helper}\ntransceiverClarityAppearance()`);
    expect(Object.keys(appearance).sort()).toEqual([
      'spectrumTraceColor', 'spectrumEnhancedColors', 'spectrumTraceFill', 'spectrumTraceSmoothing',
      'spectrumPeakGlow', 'spectrumGlassSheen', 'waterfallPalette',
      'waterfallContrast', 'waterfallSmoothing', 'showGrid', 'showCenterLine',
      'showBandEdges',
    ].sort());
    const original = Object.fromEntries(Object.keys(appearance).map(key => [key, `original-${key}`]));
    const state = { ...original, spectrumAverage: 5, sampleRate: 384000, audioStreaming: true };
    const handlers: Record<string, () => void> = {};
    const nodes: Record<string, any> = {};
    const refresh = vi.fn();
    runInNewContext(helper + binding, {
      state, waterfallRenderer: { smoothedBins: new Float32Array(2) },
      syncSetupMenuFields: vi.fn(), updateDisplayDecorations: vi.fn(),
      refreshSpectrumAppearance: refresh,
      $: (id: string) => nodes[id] ??= {
        disabled: true, addEventListener: (_event: string, fn: () => void) => { handlers[id] = fn; },
      },
    });
    handlers['display-transceiver-clarity']!();
    handlers['display-transceiver-clarity']!();
    expect(state).toEqual({ ...original, ...appearance, spectrumAverage: 5, sampleRate: 384000, audioStreaming: true });
    expect(appearance.spectrumTraceSmoothing).toBe(0);
    expect(appearance.waterfallSmoothing).toBe(0);
    expect(appearance.waterfallPalette).toBe('enhanced');
    expect(appearance.spectrumEnhancedColors).toBe(true);
    expect(nodes['display-appearance-undo'].disabled).toBe(false);
    handlers['display-appearance-undo']!();
    expect(state).toEqual({ ...original, spectrumAverage: 5, sampleRate: 384000, audioStreaming: true });
    expect(nodes['display-appearance-undo'].disabled).toBe(true);
    expect(refresh).toHaveBeenCalledTimes(3);
  });
});
