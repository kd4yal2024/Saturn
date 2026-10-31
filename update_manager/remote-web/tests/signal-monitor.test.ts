import { describe, it, expect } from 'vitest';
import { parseSignalDiagnostics, signalSummaryText } from '../src/puresignal/signal-monitor';
const report = () => ({ format: 'saturn-signal-diagnostics-v1', session: '18446744073709551614',
  generation: '11', config: '13', txEpoch: '17', point: 'PreWdspPcm', backend: 'DirectXdma', event: 'Samples',
  timeDomain: 'OfflineFixture', observedFirstNs: '100', observedLastNs: '200', recordedNs: '300',
  rateHz: 48000, analysisAvailable: true, componentPeak: .3, rms: .158, dcMagnitude: .0001,
  silent: false, componentsAtFullScale: 0, toneHz: [700, 1900], toneAmplitude: [.2, .1], toneEstimatesResolved: true,
  continuity: 'Clear', overload: 'Unknown', keyedSnapshot: false, requestedBytes: null, completedBytes: null,
  rfOutputVerified: false, rfImdQualified: false, continuousKeyedStateVerified: false, deliveryAgeVerified: false });
describe('read-only PS signal Monitor', () => {
  it('labels imported levels, origin, age and unknown overload honestly', () => {
    const text = signalSummaryText(parseSignalDiagnostics(JSON.stringify([report()])));
    for (const label of ['NOT live', 'age unverified', 'overload Unknown', '700 Hz:', '1900 Hz:', '18446744073709551614'])
      expect(text).toContain(label);
  });
  it('rejects empty evidence and qualification claims', () => {
    for (const rows of [[], [{ ...report(), rfImdQualified: true }], [{ ...report(), deliveryAgeVerified: true }]])
      expect(() => parseSignalDiagnostics(JSON.stringify(rows))).toThrow();
  });
  it('does not confuse a socket write with DMA or continuous physical RF', () => {
    const r = { ...report(), backend: 'P2', point: 'Timeline', event: 'OutputCompleted', analysisAvailable: false,
      analysisReason: 'not a sample record', requestedBytes: '1444', completedBytes: '1444', keyedSnapshot: true };
    expect(signalSummaryText(parseSignalDiagnostics(JSON.stringify([r])))).toContain('socket completion, not DMA');
  });
  it('rejects invalid levels, stale-looking timestamp substitutions and oversized input', () => {
    for (const r of [{ ...report(), componentPeak: -1 }, { ...report(), observedLastNs: '400' },
      { ...report(), generation: 'NaN' }, { ...report(), toneAmplitude: [1] }])
      expect(() => parseSignalDiagnostics(JSON.stringify([r]))).toThrow();
    expect(() => parseSignalDiagnostics(' '.repeat(4_000_001))).toThrow();
  });
  it('retains unavailable reasons without inventing levels', () => {
    const r = { ...report(), analysisAvailable: false, analysisReason: 'invalid samples/rate/scaling/tones' };
    expect(signalSummaryText(parseSignalDiagnostics(JSON.stringify([r])))).toContain('Analysis unavailable: invalid samples');
  });
  it('does not display unresolved short-window estimates as measured tone levels', () => {
    const text = signalSummaryText(parseSignalDiagnostics(JSON.stringify([{ ...report(), toneEstimatesResolved: false }])));
    expect(text).toContain('Tone estimates unresolved'); expect(text).not.toContain('700 Hz:');
  });
});
