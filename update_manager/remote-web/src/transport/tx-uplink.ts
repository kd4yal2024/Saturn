export const TX_UPLINK_DEFAULT_RTT_MS = 200;
export const TX_UPLINK_DEGRADED_RTT_FACTOR = 2;
export const TX_UPLINK_DROP_RTT_FACTOR = 4;
export const TX_UPLINK_MIN_THRESHOLD_BYTES = 1024;

export type TxUplinkDecision = {
  action: 'send' | 'drop';
  degraded: boolean;
  bufferedBytes: number;
  degradedThresholdBytes: number;
  dropThresholdBytes: number;
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

  return {
    action: bufferedBytes > dropThresholdBytes ? 'drop' : 'send',
    degraded: bufferedBytes > degradedThresholdBytes,
    bufferedBytes,
    degradedThresholdBytes,
    dropThresholdBytes,
  };
}
