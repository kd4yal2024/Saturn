export const RX_AUDIO_FRAME_FLOATS = 2048;
export const RX_AUDIO_START_PREROLL_SEC = 0.008;
export const RX_AUDIO_RECOVERY_PREROLL_SEC = 0.002;
export const RX_AUDIO_RESYNC_PREROLL_SEC = 0.004;
export const RX_AUDIO_OVERFLOW_DROP_MS = 250;
export const RX_AUDIO_MOX_RECOVERY_DROP_MS = 180;
export const RX_MSG_QUEUE_MAX_PACKETS = 8;
export const TX_MIC_BLOCK_SAMPLES = 256;

export const RX_RING_FRAMES = 4096;
export const RX_RING_CHANNELS = 2;
export const RX_CONTROL_SLOTS = 8;

export const CTRL_READ_IDX = 0;
export const CTRL_WRITE_IDX = 1;
export const CTRL_GAIN_IDX = 4;
export const CTRL_MUTED_IDX = 5;

export function volumeAmplitudeFromDb(volumeDb: number, muteThresholdDb = -39.5): number {
  const clamped = Math.max(-40, Math.min(0, Number(volumeDb) || 0));
  if (clamped <= muteThresholdDb) return 0;
  return Math.pow(10, 0.05 * clamped);
}
