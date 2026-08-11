import { describe, expect, it } from 'vitest';
import {
  buildRxAudioStartCommand,
  rxAudioTransportProfile,
} from '../src/audio/rx-profile';

describe('RX audio transport profiles', () => {
  it('keeps the full-quality LAN transport', () => {
    expect(rxAudioTransportProfile('lan')).toEqual({
      mode: 'lan',
      sampleRateHz: 48_000,
      channels: 2,
      frameFloatCount: 2_048,
    });
    expect(buildRxAudioStartCommand('lan', -10)).toContain(
      'audio_stream_channels:2;audio_stream_sample_type:float32;audio_samplerate:48000;',
    );
  });

  it('requests the bandwidth-bounded WAN transport', () => {
    expect(rxAudioTransportProfile('wan')).toEqual({
      mode: 'wan',
      sampleRateHz: 12_000,
      channels: 1,
      frameFloatCount: 2_048,
    });
    expect(buildRxAudioStartCommand('wan', -7.25)).toBe(
      'audio_stream_samples:2048;audio_stream_channels:1;' +
      'audio_stream_sample_type:float32;audio_samplerate:12000;' +
      'audio_start:0;rx_volume:0,0,-7.3;',
    );
  });

  it('fails unknown modes closed to the compatible LAN profile', () => {
    expect(rxAudioTransportProfile('automatic').mode).toBe('lan');
  });
});
