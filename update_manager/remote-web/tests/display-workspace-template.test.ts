import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

describe('responsive panadapter workspace template', () => {
  it('provides an accessible persistent spectrum and waterfall divider', () => {
    expect(template.match(/id="spectrum-waterfall-resizer"/g)).toHaveLength(1);
    expect(template).toContain('role="separator"');
    expect(template).toContain('aria-orientation="horizontal"');
    expect(template).toContain('bindDisplayWorkspaceResizer();');
    expect(template).toContain('new ResizeObserver(scheduleDisplaySurfaceResize)');
  });

  it('keeps layout preferences separate from radio and display profiles', () => {
    expect(template).toContain('const PHONE_SPECTRUM_MODE_KEY = "saturn.remote.phoneSpectrumMode"');
    expect(template).toContain('const SPECTRUM_WATERFALL_RATIO_KEY = "saturn.remote.spectrumWaterfallRatio"');
    expect(template).toContain('state.phoneSpectrumMode = nextPhoneSpectrumMode(state.phoneSpectrumMode)');
    expect(template).toContain('state.spectrumWaterfallRatio = loadSpectrumWaterfallRatio()');
  });

  it('supports deliberate touch tuning, pinch zoom, double-tap reset, and long press entry', () => {
    expect(template).toContain('if (Math.abs(dx) > Math.abs(dy) * 1.15)');
    expect(template).toContain('setDisplayZoomValue(pinchStart.zoom * scale, false, false)');
    expect(template).toContain('now - lastTap.at <= 325');
    expect(template).toContain('openFrequencyEntry(frequencyEntrySeedFromHz(targetHz))');
    expect(template).toContain('element.style.touchAction = state.frequencyLock ? "pan-y pinch-zoom" : "pan-y"');
  });
});
