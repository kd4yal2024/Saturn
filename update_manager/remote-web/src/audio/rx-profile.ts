export type RxTransportMode = 'lan' | 'wan';

export type RxAudioTransportProfile = {
  mode: RxTransportMode;
  sampleRateHz: number;
  channels: number;
  frameFloatCount: number;
};

export const RX_AUDIO_LAN_PROFILE: RxAudioTransportProfile = Object.freeze({
  mode: 'lan',
  sampleRateHz: 48_000,
  channels: 2,
  frameFloatCount: 2_048,
});

export const RX_AUDIO_WAN_PROFILE: RxAudioTransportProfile = Object.freeze({
  mode: 'wan',
  sampleRateHz: 12_000,
  channels: 1,
  // The bridge packetizer remains at 2,048 source float samples. Shaping the
  // 48 kHz stereo source to 12 kHz mono produces 256 transport samples while
  // preserving the same approximately 21 ms packet cadence.
  frameFloatCount: 2_048,
});

export function rxAudioTransportProfile(mode: unknown): RxAudioTransportProfile {
  return String(mode || '').trim().toLowerCase() === 'wan'
    ? RX_AUDIO_WAN_PROFILE
    : RX_AUDIO_LAN_PROFILE;
}

export function buildRxAudioStartCommand(
  mode: unknown,
  rxVolumeDb: number,
): string {
  const profile = rxAudioTransportProfile(mode);
  const volume = Number.isFinite(rxVolumeDb) ? rxVolumeDb : -10;
  return [
    `audio_stream_samples:${profile.frameFloatCount};`,
    `audio_stream_channels:${profile.channels};`,
    'audio_stream_sample_type:float32;',
    `audio_samplerate:${profile.sampleRateHz};`,
    'audio_start:0;',
    `rx_volume:0,0,${volume.toFixed(1)};`,
  ].join('');
}
