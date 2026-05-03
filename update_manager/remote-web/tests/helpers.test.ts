import { describe, expect, it } from 'vitest';
import { volumeAmplitudeFromDb } from '../src/audio/constants';
import { adcLabel, antennaLabel, normalizeStreamMode, noiseReductionLabel, describeMicCaptureError } from '../src/radio/band';
import { snapTuneFrequencyHz, formatFrequencyPlain, safeFiniteNumber, safeFixed } from '../src/radio/frequency';
import { clickTuneCarrierOffsetHz } from '../src/radio/passband';

describe('volumeAmplitudeFromDb', () => {
  it('returns 0 at mute threshold', () => {
    expect(volumeAmplitudeFromDb(-40)).toBe(0);
  });
  it('returns 1 at 0 dB', () => {
    expect(volumeAmplitudeFromDb(0)).toBeCloseTo(1, 5);
  });
  it('returns ~0.5 at -6 dB', () => {
    expect(volumeAmplitudeFromDb(-6)).toBeCloseTo(0.5012, 3);
  });
});

describe('adcLabel', () => {
  it('returns ADC1 for 0', () => expect(adcLabel(0)).toBe('ADC1'));
  it('returns ADC2 for 1', () => expect(adcLabel(1)).toBe('ADC2'));
  it('returns ADC3 for 2', () => expect(adcLabel(2)).toBe('ADC3'));
});

describe('antennaLabel', () => {
  it('returns ANT1 for 1', () => expect(antennaLabel(1)).toBe('ANT1'));
  it('returns ANT3 for 3', () => expect(antennaLabel(3)).toBe('ANT3'));
});

describe('normalizeStreamMode', () => {
  it('returns lan by default', () => expect(normalizeStreamMode('')).toBe('lan'));
  it('recognizes wan', () => expect(normalizeStreamMode('WAN')).toBe('wan'));
  it('returns lan for garbage', () => expect(normalizeStreamMode('xyz')).toBe('lan'));
});

describe('snapTuneFrequencyHz', () => {
  it('snaps to 10 Hz by default', () => expect(snapTuneFrequencyHz(7150003)).toBe(7150000));
  it('snaps to 100 Hz step', () => expect(snapTuneFrequencyHz(7150060, 100)).toBe(7150100));
  it('returns 0 for negative', () => expect(snapTuneFrequencyHz(-100)).toBe(0));
});

describe('formatFrequencyPlain', () => {
  it('formats 7.150.000', () => expect(formatFrequencyPlain(7150000)).toBe('7.150.000'));
  it('formats 14.225.500', () => expect(formatFrequencyPlain(14225500)).toBe('14.225.500'));
});

describe('safeFiniteNumber', () => {
  it('returns number for valid input', () => expect(safeFiniteNumber('42')).toBe(42));
  it('returns fallback for NaN', () => expect(safeFiniteNumber('abc', 5)).toBe(5));
  it('returns fallback for null', () => expect(safeFiniteNumber(null)).toBe(0));
});

describe('safeFixed', () => {
  it('formats valid number', () => expect(safeFixed(3.14159, 2, '?')).toBe('3.14'));
  it('returns fallback for NaN', () => expect(safeFixed('abc', 1, 'N/A')).toBe('N/A'));
});

describe('noiseReductionLabel', () => {
  it('returns Off for OFF', () => expect(noiseReductionLabel('OFF', 100)).toBe('Off'));
  it('returns NR3 fixed for NR3', () => expect(noiseReductionLabel('NR3', 50)).toBe('NR3 fixed'));
  it('returns mode + level for NR2', () => expect(noiseReductionLabel('NR2', 80)).toBe('NR2 80%'));
});

describe('describeMicCaptureError', () => {
  it('returns denied message for NotAllowedError on secure context', () => {
    const msg = describeMicCaptureError({ name: 'NotAllowedError' }, true);
    expect(msg).toContain('denied');
  });
  it('returns HTTPS message for NotAllowedError on insecure context', () => {
    const msg = describeMicCaptureError({ name: 'NotAllowedError' }, false);
    expect(msg).toContain('HTTPS');
  });
  it('returns not found for NotFoundError', () => {
    const msg = describeMicCaptureError({ name: 'NotFoundError' }, true);
    expect(msg).toContain('No microphone');
  });
  it('returns error message if available', () => {
    const msg = describeMicCaptureError({ name: 'SomeError', message: 'Custom msg' }, true);
    expect(msg).toBe('Custom msg');
  });
  it('returns fallback for unknown error', () => {
    const msg = describeMicCaptureError({}, true);
    expect(msg).toBe('Unknown microphone capture failure');
  });
});

describe('clickTuneCarrierOffsetHz', () => {
  it('returns 0 for AM', () => expect(clickTuneCarrierOffsetHz(50, 3050, 'AM')).toBe(0));
  it('returns 0 for USB (not offset mode)', () => expect(clickTuneCarrierOffsetHz(50, 3050, 'USB')).toBe(0));
  it('returns negative midpoint for LSB', () => {
    const offset = clickTuneCarrierOffsetHz(50, 3050, 'LSB');
    expect(offset).toBe(-1550);
  });
  it('returns positive midpoint for DIGU', () => {
    const offset = clickTuneCarrierOffsetHz(300, 3000, 'DIGU');
    expect(offset).toBe(1650);
  });
});
