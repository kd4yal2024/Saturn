import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(resolve(process.cwd(), '../templates/p23test.html'), 'utf8');
const begin = template.indexOf('/* RX_COUNTER_V31_PRESENTATION_BEGIN */');
const end = template.indexOf('/* RX_COUNTER_V31_PRESENTATION_END */');
const context = createContext({});
runInContext(template.slice(begin, end), context);
const present = context.rxCounterV31Presentation as (value?: Record<string, unknown>, activeBackend?: string) =>
  { state: string; summary: string; detail: string };

describe('RXC1 receiver-counter presentation', () => {
  it('never turns missing or failed acquisition into zero loss', () => {
    expect(present().detail).toContain('no loss count is inferred');
    expect(present({ schema: 'rxc1-v1', source_backend: 'xdma', status: 'read_error' }).state)
      .toBe('unavailable');
  });

  it('shows disabled polling as unavailable without hiding other telemetry', () => {
    const result = present({ schema: 'rxc1-v1', source_backend: 'xdma', status: 'disabled' }, 'xdma');
    expect(result.state).toBe('unavailable');
    expect(result.summary).toBe('disabled / unavailable');
    expect(result.detail).toContain('no register reads or loss count');
  });

  it('does not attribute a stale P2 reading to a direct-XDMA session', () => {
    const result = present({ schema: 'rxc1-v1', source_backend: 'p2', status: 'valid' }, 'xdma');
    expect(result.state).toBe('unavailable');
    expect(result.detail).toContain('does not belong to active xdma backend');
  });

  it('rejects a nominally valid but incomplete snapshot', () => {
    const result = present({
      schema: 'rxc1-v1', source_backend: 'p2', status: 'partial',
      sampled_at_ms: Date.now(),
      host_acquisition_failures: 1,
      ddc: [{ receiver: 6, status: 'valid', refused_pre_fir_pair_candidates: '0' }],
    });
    expect(result.summary).toContain('0/10 DDC snapshots valid');
    expect(result.detail).toContain('DDC6 unavailable (malformed or mixed-session snapshot)');
  });

  it('shows a real zero only for a complete, exact snapshot', () => {
    const result = present({
      schema: 'rxc1-v1', source_backend: 'p2', status: 'partial',
      sampled_at_ms: Date.now(),
      host_acquisition_failures: 0,
      ddc: [{
        receiver: 0, status: 'valid', snapshot_serial: 1, host_token: 22,
        session_generation: 0, configuration_generation: 1, rate_code: 6,
        observed_configuration_word: '0x00000000000000', overflow: false, exact: true,
        accepted_pre_fir_pairs: '50', refused_pre_fir_pair_candidates: '0',
        accepted_clamped_iq_components: '0', partial_ready_anomalies: '0',
        one_sided_valid_anomalies: '0',
      }],
    });
    expect(result.summary).toContain('1/10 DDC snapshots valid');
    expect(result.detail).toContain('DDC0 pre-FIR pairs not accepted 0');
    expect(result.detail).toContain('exact');
  });

  it('preserves decimal-string counter precision and provenance', () => {
    const result = present({
      schema: 'rxc1-v1', source_backend: 'xdma', status: 'partial',
      sampled_at_ms: Date.now(),
      host_acquisition_failures: 0,
      ddc: [{
        receiver: 0, status: 'valid', snapshot_serial: 9, host_token: 123,
        session_generation: 7, configuration_generation: 3, rate_code: 6,
        observed_configuration_word: '0x00000000000000', overflow: true, exact: false,
        accepted_pre_fir_pairs: '18446744073709551615',
        refused_pre_fir_pair_candidates: '0',
        accepted_clamped_iq_components: '2', partial_ready_anomalies: '0',
        one_sided_valid_anomalies: '0',
      }, { receiver: 1, status: 'reset_changed', refused_pre_fir_pair_candidates: null }],
    });
    expect(result.summary).toContain('xdma acquisition · 1/10');
    expect(result.detail).toContain('18446744073709551615');
    expect(result.detail).toContain('DDC1 unavailable (reset_changed)');
    expect(result.detail).toContain('saturated; inexact');
  });

  it('shows nonzero drops and clamps from distinct receivers, but rejects mixed sessions', () => {
    const entry = (receiver: number, serial: number, token = 123) => ({
      receiver, status: 'valid', snapshot_serial: serial, host_token: token,
      session_generation: 7, configuration_generation: 3, rate_code: 4,
      observed_configuration_word: '0x0000000001', overflow: false, exact: true,
      accepted_pre_fir_pairs: String(100 + receiver),
      refused_pre_fir_pair_candidates: String(3 + receiver),
      accepted_clamped_iq_components: String(7 + receiver),
      partial_ready_anomalies: '1', one_sided_valid_anomalies: '2',
    });
    const sample = { schema: 'rxc1-v1', source_backend: 'xdma', status: 'partial',
      sampled_at_ms: Date.now(), host_acquisition_failures: 0,
      ddc: [entry(0, 10), entry(6, 16)] };
    const good = present(sample, 'xdma');
    expect(good.summary).toContain('2/10 DDC snapshots valid');
    expect(good.detail).toContain('DDC6 pre-FIR pairs not accepted 9');
    expect(good.detail).toContain('accepted clamped I/Q components 13');
    const mixed = present({ ...sample, ddc: [entry(0, 10), entry(6, 16, 999)] }, 'xdma');
    expect(mixed.summary).toContain('0/10 DDC snapshots valid');
    expect(mixed.detail).toContain('mixed-session');
    const serial = present({ ...sample, ddc: [entry(0, 16), entry(6, 10)] }, 'xdma');
    expect(serial.summary).toContain('0/10 DDC snapshots valid');
  });

  it('rejects a stale same-backend zero and a missing timestamp', () => {
    const sample = {
      schema: 'rxc1-v1', source_backend: 'xdma', status: 'partial',
      ddc: [{
        receiver: 0, status: 'valid', snapshot_serial: 1, host_token: 22,
        session_generation: 0, configuration_generation: 1, rate_code: 6,
        observed_configuration_word: '0x00000000000000', overflow: false, exact: true,
        accepted_pre_fir_pairs: '50', refused_pre_fir_pair_candidates: '0',
        accepted_clamped_iq_components: '0', partial_ready_anomalies: '0',
        one_sided_valid_anomalies: '0',
      }],
    };
    expect(present({ ...sample, sampled_at_ms: Date.now() - 31000 }, 'xdma').state)
      .toBe('unavailable');
    expect(present(sample, 'xdma').detail).toContain('timestamp is missing');
  });

  it('is connected to the existing telemetry page without a second poller', () => {
    expect(template).toContain('id="p23-rx-counter-v31-summary"');
    expect(template).toContain('updateRxCounterV31Summary(perf);');
    expect(template).toContain("runtimeRow('Receiver counter snapshots · RXC1'");
  });
});
