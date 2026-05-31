export const TX_UPLINK_DEFAULT_RTT_MS = 200;
export const TX_UPLINK_DEGRADED_RTT_FACTOR = 2;
export const TX_UPLINK_DROP_RTT_FACTOR = 4;
export const TX_UPLINK_MIN_THRESHOLD_BYTES = 1024;
// Phase 41 stopgap: absolute ceilings on browser WebSocket.bufferedAmount for
// the TX mic path. Cell/VPN links can have low bridge RTT but bursty upstream
// scheduling, so the RTT-scaled thresholds are telemetry only; real mic frames
// are dropped only at the absolute hard cap. Phase 42's control lane still owns
// RF-off and flushes late media, so bounded buffering is preferable to chopping
// PCM on slow links until Phase 44 Opus lands.
export const TX_UPLINK_SOFT_CAP_BYTES = 32_768;
export const TX_UPLINK_HARD_CAP_BYTES = 65_536;

export type TxUplinkDecision = {
  action: 'send' | 'drop';
  degraded: boolean;
  bufferedBytes: number;
  degradedThresholdBytes: number;
  dropThresholdBytes: number;
  softCapEngaged: boolean;
  hardCapEngaged: boolean;
};

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
): TxUplinkDecision {
  const bufferedBytes = Math.max(0, Math.round(Number.isFinite(bufferedAmountBytes) ? bufferedAmountBytes : 0));
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

  const softCapEngaged = bufferedBytes > TX_UPLINK_SOFT_CAP_BYTES;
  const hardCapEngaged = bufferedBytes > TX_UPLINK_HARD_CAP_BYTES;
  return {
    action: hardCapEngaged ? 'drop' : 'send',
    degraded: softCapEngaged || hardCapEngaged,
    bufferedBytes,
    degradedThresholdBytes,
    dropThresholdBytes,
    softCapEngaged,
    hardCapEngaged,
  };
}
