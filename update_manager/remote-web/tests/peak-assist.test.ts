import { describe, expect, it } from 'vitest';
import {
  detectPeakInPassband,
  emptyPeakAssistTrackingState,
  trackPeakAssist,
} from '../src/dsp/peak-assist';

describe('detectPeakInPassband', () => {
  it('ignores stronger signals outside the receive passband', () => {
    const bins = new Float32Array(101).fill(-120);
    bins[20] = -20;
    bins[52] = -45;
    const result = detectPeakInPassband(bins, 14_200_000, 100_000, 0, 3_000);
    expect(result).not.toBeNull();
    expect(result?.bin).toBeCloseTo(52, 1);
    expect(result?.levelDb).toBe(-45);
  });

  it('uses parabolic refinement around the strongest bin', () => {
    const bins = new Float32Array(101).fill(-120);
    bins[49] = -60;
    bins[50] = -40;
    bins[51] = -50;
    const result = detectPeakInPassband(bins, 7_100_000, 10_100, -1_000, 1_000);
    expect(result).not.toBeNull();
    expect(result?.bin).toBeGreaterThan(50);
    expect(result?.bin).toBeLessThan(50.5);
  });

  it('returns null when the passband does not intersect usable bins', () => {
    const bins = new Float32Array(64).fill(-100);
    expect(detectPeakInPassband(bins, 1_000_000, 48_000, 40_000, 50_000)).toBeNull();
  });

  it('rejects a strongest bin that is not distinct from the passband noise floor', () => {
    const bins = new Float32Array(101).fill(-100);
    bins[51] = -96;
    expect(detectPeakInPassband(bins, 14_200_000, 100_000, 0, 3_000)).toBeNull();
  });
});

describe('trackPeakAssist', () => {
  const peak = (frequencyHz: number, levelDb = -40) => ({
    bin: 50,
    frequencyHz,
    levelDb,
    prominenceDb: 20,
  });

  it('requires three consistent frames before locking a peak', () => {
    let state = emptyPeakAssistTrackingState();
    for (let frame = 1; frame <= 2; frame += 1) {
      const tracked = trackPeakAssist(state, peak(7_101_500 + frame * 5), 25);
      state = tracked.state;
      expect(tracked.peak).toBeNull();
    }
    const tracked = trackPeakAssist(state, peak(7_101_510), 25);
    expect(tracked.peak?.frequencyHz).toBeCloseTo(7_101_510, 0);
  });

  it('does not jump to a transient distant peak', () => {
    let state = emptyPeakAssistTrackingState();
    for (let frame = 0; frame < 3; frame += 1) {
      state = trackPeakAssist(state, peak(7_101_500), 25).state;
    }
    const tracked = trackPeakAssist(state, peak(7_102_600), 25);
    expect(tracked.peak?.frequencyHz).toBeCloseTo(7_101_500, 0);
  });

  it('releases a locked peak after sustained signal loss', () => {
    let state = emptyPeakAssistTrackingState();
    for (let frame = 0; frame < 3; frame += 1) {
      state = trackPeakAssist(state, peak(7_101_500), 25).state;
    }
    let tracked = { state, peak: state.locked };
    for (let frame = 0; frame < 7; frame += 1) {
      tracked = trackPeakAssist(tracked.state, null, 25);
    }
    expect(tracked.peak).toBeNull();
  });
});
