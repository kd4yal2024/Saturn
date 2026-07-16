export const TX_UPLINK_DEFAULT_RTT_MS = 200;
export const TX_UPLINK_DEGRADED_RTT_FACTOR = 2;
export const TX_UPLINK_DROP_RTT_FACTOR = 4;
export const TX_UPLINK_MIN_THRESHOLD_BYTES = 1024;
// Absolute ceilings on browser WebSocket.bufferedAmount for
// the TX mic path. Cell/VPN links can have low bridge RTT but bursty upstream
// scheduling, so the RTT-scaled thresholds are telemetry only; real mic frames
// are dropped only at the absolute hard cap. The split control lane still owns
// RF-off and flushes late media, so bounded buffering is preferable to chopping
// PCM fallback on slow links when Opus is unavailable.
export const TX_UPLINK_SOFT_CAP_BYTES = 32_768;
export const TX_UPLINK_PCM_HARD_CAP_BYTES = 65_536;
export const TX_UPLINK_OPUS_HARD_CAP_BYTES = 4_096;
export const TX_UPLINK_HARD_CAP_BYTES = TX_UPLINK_PCM_HARD_CAP_BYTES;
export const TX_MIC_FRAME_HEADER_BYTES = 64;
export const TX_MIC_SAMPLE_RATE_HZ = 48_000;
export const TX_MIC_SAMPLE_TYPE_S16 = 1;
export const TX_MIC_BYTES_PER_SAMPLE_S16 = 2;
export const TX_MIC_STREAM_TYPE = 2;
export const TX_MIC_CODEC_PCM = 0;
export const TX_MIC_CODEC_OPUS_NB = 1;
export const TX_MIC_CODEC_OPUS_WB = 2;

export type TxCodecCapability = 'pcm' | 'opus_nb' | 'opus_wb';

export type TxCodecCapabilityDetection = {
  detected: TxCodecCapability[];
  advertised: TxCodecCapability[];
  webCodecsAudioEncoder: boolean;
  opusNb: boolean;
  opusWb: boolean;
  opusNbProbe: string;
  opusWbProbe: string;
};

export type TxCodecCapabilityDetectionOptions = {
  advertiseOpus?: boolean;
};

type AudioEncoderConstructorLike = {
  isConfigSupported?: (config: Record<string, unknown>) => Promise<{ supported?: boolean }>;
};

async function audioEncoderSupports(
  encoder: AudioEncoderConstructorLike,
  config: Record<string, unknown>,
): Promise<{ supported: boolean; error?: string }> {
  if (typeof encoder.isConfigSupported !== 'function') {
    return { supported: false, error: 'missing_isConfigSupported' };
  }
  try {
    const result = await encoder.isConfigSupported(config);
    return { supported: result?.supported === true };
  } catch (error) {
    return {
      supported: false,
      error: error instanceof Error ? error.name || error.message : 'exception',
    };
  }
}

type OpusProbe = {
  label: string;
  config: Record<string, unknown>;
};

function opusEncoderProbes(sampleRate: number, bitrate: number): OpusProbe[] {
  const base = {
    codec: 'opus',
    sampleRate,
    numberOfChannels: 1,
    bitrate,
  };
  return [
    {
      label: 'minimal',
      config: base,
    },
    {
      label: 'full-voice-opus-fec',
      config: {
        ...base,
        opus: {
          format: 'opus',
          signal: 'voice',
          application: 'voip',
          frameDuration: 20_000,
          complexity: 5,
          packetlossperc: 5,
          useinbandfec: true,
          usedtx: false,
        },
      },
    },
    {
      label: 'full-voice-opus-no-fec',
      config: {
        ...base,
        opus: {
          format: 'opus',
          signal: 'voice',
          application: 'voip',
          frameDuration: 20_000,
          complexity: 5,
          packetlossperc: 0,
          useinbandfec: false,
          usedtx: false,
        },
      },
    },
    {
      label: 'full-audio-opus',
      config: {
        ...base,
        opus: {
          format: 'opus',
          signal: 'auto',
          application: 'audio',
          frameDuration: 20_000,
          usedtx: false,
        },
      },
    },
  ];
}

async function detectOpusEncoderSupport(
  encoder: AudioEncoderConstructorLike,
  sampleRate: number,
  bitrate: number,
): Promise<{ supported: boolean; probe: string }> {
  const rejected: string[] = [];
  for (const probe of opusEncoderProbes(sampleRate, bitrate)) {
    const result = await audioEncoderSupports(encoder, probe.config);
    if (result.supported) {
      return { supported: true, probe: probe.label };
    }
    rejected.push(result.error ? `${probe.label}:${result.error}` : probe.label);
  }
  return { supported: false, probe: `rejected(${rejected.join('|')})` };
}

export async function detectTxCodecCapabilities(
  scope: unknown = globalThis,
  options: TxCodecCapabilityDetectionOptions = {},
): Promise<TxCodecCapabilityDetection> {
  const encoder = (scope as { AudioEncoder?: AudioEncoderConstructorLike } | null | undefined)?.AudioEncoder;
  const detected: TxCodecCapability[] = ['pcm'];

  if (!encoder || typeof encoder.isConfigSupported !== 'function') {
    return {
      detected,
      advertised: ['pcm'],
      webCodecsAudioEncoder: false,
      opusNb: false,
      opusWb: false,
      opusNbProbe: 'no-audioencoder',
      opusWbProbe: 'no-audioencoder',
    };
  }

  const [opusNbResult, opusWbResult] = await Promise.all([
    detectOpusEncoderSupport(encoder, 16_000, 16_000),
    detectOpusEncoderSupport(encoder, 48_000, 24_000),
  ]);
  const opusNb = opusNbResult.supported;
  const opusWb = opusWbResult.supported;

  if (opusNb) detected.push('opus_nb');
  if (opusWb) detected.push('opus_wb');

  return {
    detected,
    // Keep source behavior unchanged until the bridge Opus backend
    // and browser/bridge force-RX/fallback acceptance tests are ready.
    // The first browser integration gate only supports wideband from the
    // 48 kHz mic path; narrowband needs an explicit browser-side resampler.
    advertised: options.advertiseOpus === true
      ? detected.filter((codec) => codec !== 'opus_nb')
      : ['pcm'],
    webCodecsAudioEncoder: true,
    opusNb,
    opusWb,
    opusNbProbe: opusNbResult.probe,
    opusWbProbe: opusWbResult.probe,
  };
}

export type TxUplinkDecision = {
  action: 'send' | 'drop';
  degraded: boolean;
  bufferedBytes: number;
  degradedThresholdBytes: number;
  dropThresholdBytes: number;
  hardCapBytes: number;
  softCapEngaged: boolean;
  hardCapEngaged: boolean;
};

export function txUplinkHardCapBytesForCodec(codec: TxCodecCapability | number | null | undefined): number {
  if (codec === TX_MIC_CODEC_OPUS_NB || codec === TX_MIC_CODEC_OPUS_WB) {
    return TX_UPLINK_OPUS_HARD_CAP_BYTES;
  }
  if (typeof codec === 'string') {
    const normalized = codec.trim().toLowerCase();
    if (normalized === 'opus_nb' || normalized === 'opus_wb') {
      return TX_UPLINK_OPUS_HARD_CAP_BYTES;
    }
  }
  return TX_UPLINK_PCM_HARD_CAP_BYTES;
}

export function txMicByteRateBytesPerSecond(
  sampleRateHz = 48_000,
  bytesPerSample = 4,
  blockSamples = 256,
  headerBytes = 64,
): number {
  const sampleRate = Math.max(1, Number.isFinite(sampleRateHz) ? sampleRateHz : 48_000);
  const payloadBytesPerSecond = sampleRate * Math.max(1, bytesPerSample);
  const framesPerSecond = sampleRate / Math.max(1, blockSamples);
  return payloadBytesPerSecond + framesPerSecond * Math.max(0, headerBytes);
}

export type TxMicPcmFrame = {
  frame: ArrayBuffer;
  peak: number;
  payloadBytes: number;
};

export function buildTxMicPcmS16Frame(
  monoSamples: ArrayLike<number>,
  sequence: number,
  sampleRateHz = TX_MIC_SAMPLE_RATE_HZ,
): TxMicPcmFrame {
  const sampleCount = Math.max(0, Math.floor(Number(monoSamples.length) || 0));
  const payloadBytes = sampleCount * TX_MIC_BYTES_PER_SAMPLE_S16;
  const frame = new ArrayBuffer(TX_MIC_FRAME_HEADER_BYTES + payloadBytes);
  const view = new DataView(frame);
  view.setUint32(0, 0, true);
  view.setUint32(4, Math.max(1, Math.round(Number(sampleRateHz) || TX_MIC_SAMPLE_RATE_HZ)), true);
  view.setUint32(8, TX_MIC_SAMPLE_TYPE_S16, true);
  view.setUint32(20, sampleCount, true);
  view.setUint32(24, TX_MIC_STREAM_TYPE, true);
  view.setUint32(28, 1, true);
  view.setUint32(32, sequence >>> 0, true);
  view.setUint32(36, TX_MIC_CODEC_PCM, true);
  view.setUint32(40, payloadBytes >>> 0, true);

  let peak = 0;
  for (let i = 0; i < sampleCount; i += 1) {
    const sample = Number(monoSamples[i]) || 0;
    const clamped = Math.max(-1, Math.min(1, sample));
    peak = Math.max(peak, Math.abs(clamped));
    const pcm = clamped < 0 ? Math.round(clamped * 32768) : Math.round(clamped * 32767);
    view.setInt16(
      TX_MIC_FRAME_HEADER_BYTES + i * TX_MIC_BYTES_PER_SAMPLE_S16,
      Math.max(-32768, Math.min(32767, pcm)),
      true,
    );
  }

  return { frame, peak, payloadBytes };
}

export function txUplinkBufferedThresholdBytes(
  rttMs: number | null | undefined,
  micByteRateBytesPerSecond: number,
  rttFactor: number,
): number {
  const effectiveRttMs =
    rttMs != null && Number.isFinite(rttMs) && rttMs > 0 ? rttMs : TX_UPLINK_DEFAULT_RTT_MS;
  const byteRate = Math.max(1, Number.isFinite(micByteRateBytesPerSecond) ? micByteRateBytesPerSecond : 1);
  const factor = Math.max(0, Number.isFinite(rttFactor) ? rttFactor : 0);
  return Math.max(TX_UPLINK_MIN_THRESHOLD_BYTES, Math.round((byteRate * effectiveRttMs * factor) / 1000));
}

export function decideTxMicSend(
  bufferedAmountBytes: number,
  rttMs: number | null | undefined,
  micByteRateBytesPerSecond: number,
  codec: TxCodecCapability | number | null | undefined = 'pcm',
): TxUplinkDecision {
  const bufferedBytes = Math.max(0, Math.round(Number.isFinite(bufferedAmountBytes) ? bufferedAmountBytes : 0));
  const hardCapBytes = txUplinkHardCapBytesForCodec(codec);
  const degradedThresholdBytes = txUplinkBufferedThresholdBytes(
    rttMs,
    micByteRateBytesPerSecond,
    TX_UPLINK_DEGRADED_RTT_FACTOR,
  );
  const dropThresholdBytes = txUplinkBufferedThresholdBytes(
    rttMs,
    micByteRateBytesPerSecond,
    TX_UPLINK_DROP_RTT_FACTOR,
  );

  const softCapEngaged = bufferedBytes > Math.min(TX_UPLINK_SOFT_CAP_BYTES, hardCapBytes);
  const hardCapEngaged = bufferedBytes > hardCapBytes;
  return {
    action: hardCapEngaged ? 'drop' : 'send',
    degraded: softCapEngaged || hardCapEngaged,
    bufferedBytes,
    degradedThresholdBytes,
    dropThresholdBytes,
    hardCapBytes,
    softCapEngaged,
    hardCapEngaged,
  };
}
