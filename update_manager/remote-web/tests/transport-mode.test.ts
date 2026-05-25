import { describe, expect, it } from 'vitest';
import {
  PHASE42_SPLIT_QUERY_PARAM,
  PHASE42_SPLIT_STORAGE_KEY,
  createPhase42SessionId,
  phase42SplitTransportEnabled,
} from '../src/transport/transport-mode';

describe('Phase 42 transport mode helpers', () => {
  it('requires an explicit query or persisted opt-in', () => {
    expect(phase42SplitTransportEnabled('')).toBe(false);
    expect(phase42SplitTransportEnabled('', '1')).toBe(true);
    expect(phase42SplitTransportEnabled(`?${PHASE42_SPLIT_QUERY_PARAM}=1`)).toBe(true);
    expect(phase42SplitTransportEnabled(`?${PHASE42_SPLIT_QUERY_PARAM}=true`)).toBe(true);
  });

  it('lets the query string override persisted opt-in', () => {
    expect(phase42SplitTransportEnabled(`?${PHASE42_SPLIT_QUERY_PARAM}=0`, '1')).toBe(false);
    expect(phase42SplitTransportEnabled(`?${PHASE42_SPLIT_QUERY_PARAM}=off`, 'yes')).toBe(false);
  });

  it('documents the local storage key consumed by the template', () => {
    expect(PHASE42_SPLIT_STORAGE_KEY).toBe('saturn.phase42.splitTransport');
  });

  it('creates delimiter-safe session ids', () => {
    expect(createPhase42SessionId(1_700_000_000_000, 0.5)).toBe('phase42-loyw3v28-0zik0zk');
    expect(createPhase42SessionId(Number.NaN, Number.NaN)).toBe('phase42-0-0000000');
    expect(createPhase42SessionId()).toMatch(/^phase42-[a-z0-9]+-[a-z0-9]+$/);
  });
});
