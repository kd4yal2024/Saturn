export const TX_UPLINK_DEFAULT_RTT_MS = 200;
export const TX_UPLINK_DEGRADED_RTT_FACTOR = 2;
export const TX_UPLINK_DROP_RTT_FACTOR = 4;
export const TX_UPLINK_MIN_THRESHOLD_BYTES = 1024;
// Phase 41 stopgap: absolute ceilings on browser WebSocket.bufferedAmount for
// the TX mic path. Keep 16 KB as an early degraded warning, but allow one more
// jitter window before dropping real mic frames. The Phase 42 control lane
// release path still owns RF-off and flushes late media, so a small extra TX
// audio buffer is preferable to chopping PCM on cell/VPN links.
export const TX_UPLINK_SOFT_CAP_BYTES = 16_384;
export const TX_UPLINK_HARD_CAP_BYTES = 32_768;

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
    action: hardCapEngaged || bufferedBytes > dropThresholdBytes ? 'drop' : 'send',
    degraded: softCapEngaged || hardCapEngaged || bufferedBytes > degradedThresholdBytes,
    bufferedBytes,
    degradedThresholdBytes,
    dropThresholdBytes,
    softCapEngaged,
    hardCapEngaged,
  };
}
