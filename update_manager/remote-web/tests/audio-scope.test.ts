import { describe, expect, it } from 'vitest';

import { buildAudioScopeSnapshot } from '../src/audio/scope';

describe('buildAudioScopeSnapshot', () => {
  it('builds bounded stereo waveforms and accurate peak levels', () => {
    const left = new Float32Array([0, 0.25, -0.5, 1]);
    const right = new Float32Array([0, -0.1, 0.2, -0.75]);
    const snapshot = buildAudioScopeSnapshot(left, right, 8);

    expect(snapshot.left).toHaveLength(8);
    expect(snapshot.right).toHaveLength(8);
    expect(snapshot.leftPeak).toBe(1);
    expect(snapshot.rightPeak).toBe(0.75);
    expect(snapshot.leftPeakDbfs).toBeCloseTo(0, 5);
    expect(snapshot.rightPeakDbfs).toBeCloseTo(-2.5, 1);
  });

  it('handles silence, non-finite values, and oversized samples safely', () => {
    const snapshot = buildAudioScopeSnapshot(
      new Float32Array([Number.NaN, 2, -2]),
      new Float32Array(0),
      4,
    );

    expect(snapshot.left).toHaveLength(8);
    expect(snapshot.leftPeak).toBe(1);
    expect(snapshot.rightPeak).toBe(0);
    expect(snapshot.rightPeakDbfs).toBe(-90);
    expect(Array.from(snapshot.left).every(Number.isFinite)).toBe(true);
  });
});
