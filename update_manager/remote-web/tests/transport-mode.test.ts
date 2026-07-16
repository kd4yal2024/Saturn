import { describe, expect, it } from 'vitest';
import {
  SPLIT_TRANSPORT_LEGACY_QUERY_PARAM,
  SPLIT_TRANSPORT_LEGACY_STORAGE_KEY,
  SPLIT_TRANSPORT_QUERY_PARAM,
  SPLIT_TRANSPORT_STORAGE_KEY,
  createSplitSessionId,
  splitTransportEnabled,
} from '../src/transport/transport-mode';

describe('split transport mode helpers', () => {
  it('enables split transport by default', () => {
    expect(splitTransportEnabled('')).toBe(true);
    expect(splitTransportEnabled('', '1')).toBe(true);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=1`)).toBe(true);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=true`)).toBe(true);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=split`)).toBe(true);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_LEGACY_QUERY_PARAM}=1`)).toBe(true);
  });

  it('lets the query string disable split transport for fallback testing', () => {
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=0`)).toBe(false);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=false`)).toBe(false);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=legacy`)).toBe(false);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=single`)).toBe(false);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=0`, '1')).toBe(false);
    expect(splitTransportEnabled(`?${SPLIT_TRANSPORT_QUERY_PARAM}=off`, 'yes')).toBe(false);
  });

  it('lets persisted local storage disable split transport', () => {
    expect(splitTransportEnabled('', '0')).toBe(false);
    expect(splitTransportEnabled('', 'off')).toBe(false);
    expect(splitTransportEnabled('', 'legacy')).toBe(false);
  });

  it('documents the local storage key consumed by the template', () => {
    expect(SPLIT_TRANSPORT_STORAGE_KEY).toBe('saturn.remote.splitTransport');
    expect(SPLIT_TRANSPORT_LEGACY_STORAGE_KEY).toBe('saturn.phase42.splitTransport');
  });

  it('creates delimiter-safe session ids', () => {
    expect(createSplitSessionId(1_700_000_000_000, 0.5)).toBe('split-loyw3v28-0zik0zk');
    expect(createSplitSessionId(Number.NaN, Number.NaN)).toBe('split-0-0000000');
    expect(createSplitSessionId()).toMatch(/^split-[a-z0-9]+-[a-z0-9]+$/);
  });
});
