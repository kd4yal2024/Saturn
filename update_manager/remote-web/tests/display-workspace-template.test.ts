import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);
const layoutValidator = readFileSync(
  new URL('../scripts/validate-remote-next-layout.mjs', import.meta.url),
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

  it('fits the complete appliance console into a 1920x1080 LCD viewport', () => {
    expect(template).toContain('Phase 12: fixed-height 1920x1080 appliance LCD console.');
    expect(template).toContain('@media (min-width: 1440px) and (min-height: 850px) and (max-height: 1200px)');
    expect(template).toContain('grid-template-rows: auto auto auto minmax(0, 1fr) auto;');
    expect(template).toContain('height: 174px;');
    expect(template).toContain('--display-workspace-height: auto;');
    expect(layoutValidator).toContain('lcdViewportOverflow');
    expect(layoutValidator).toContain('scenario.name === "desktop-hd"');
  });
});
