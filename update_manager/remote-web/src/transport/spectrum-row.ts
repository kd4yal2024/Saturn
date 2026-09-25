/**
 * Server-computed RX spectrum rows for WAN display clients.
 *
 * The bridge advertises `saturn_display_caps:spectrum_u8;` just before
 * `ready;` on Direct-XDMA builds. A WAN browser may then ask for spectrum
 * rows instead of the full-rate raw IQ stream; LAN clients never opt in and
 * keep byte-for-byte the existing IQ path.
 *
 * This module owns the frozen wire contract (stream type 16) plus the pure
 * negotiation helpers: clamping, command building, echo parsing and caps
 * detection. The template wires them to the socket.
 */

export const SPECTRUM_ROW_HEADER_BYTES = 64;
export const SPECTRUM_ROW_STREAM_TYPE = 16;
export const SPECTRUM_ROW_FORMAT_U8_DB = 0x5301;
export const SPECTRUM_ROW_MAX_BINS = 4096;
export const SPECTRUM_ROW_FLAG_FFTSHIFTED = 1;
export const SPECTRUM_ROW_CHANNELS = 1;

export const SPECTRUM_DISPLAY_CAPS_TOKEN = 'spectrum_u8';
export const SPECTRUM_DISPLAY_CAPS_PREFIX = 'saturn_display_caps:';
export const SPECTRUM_DISPLAY_PREFIX = 'saturn_display:';
export const SPECTRUM_DISPLAY_ACK_PREFIX = 'saturn_display_ack:';
export const SPECTRUM_DISPLAY_IQ_COMMAND = 'saturn_display:iq;';

export const SPECTRUM_FFT_MIN = 256;
export const SPECTRUM_FFT_MAX = 4096;
export const SPECTRUM_INTERVAL_MIN_MS = 33;
export const SPECTRUM_INTERVAL_MAX_MS = 250;

export type SpectrumRowHeader = {
  receiver: number;
  spanHz: number;
  format: number;
  fftSize: number;
  flags: number;
  binCount: number;
  streamType: number;
  channels: number;
  sequence: number;
  centerHz: number;
  dbOffset: number;
  dbStep: number;
  captureToEnqueueUs: number;
  serverMs: number;
};

export type DisplayEcho =
  | { mode: 'iq' }
  | { mode: 'spectrum'; fftSize: number; intervalMs: number };

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

function powerOfTwoFloor(value: number): number {
  return 1 << (31 - Math.clz32(value));
}

/** Round down to a power of two and clamp, matching the bridge exactly. */
export function clampSpectrumFftSize(requested: number): number {
  const candidate = Math.round(finiteOr(requested, SPECTRUM_FFT_MIN));
  const clamped = Math.max(SPECTRUM_FFT_MIN, Math.min(SPECTRUM_FFT_MAX, candidate));
  return powerOfTwoFloor(clamped);
}

export function clampSpectrumIntervalMs(requested: number): number {
  const candidate = Math.round(finiteOr(requested, SPECTRUM_INTERVAL_MAX_MS));
  return Math.max(SPECTRUM_INTERVAL_MIN_MS, Math.min(SPECTRUM_INTERVAL_MAX_MS, candidate));
}

export function spectrumDisplayCommand(fftSize: number, intervalMs: number): string {
  return `saturn_display:spectrum,${clampSpectrumFftSize(fftSize)},${clampSpectrumIntervalMs(intervalMs)};`;
}

export function spectrumDisplayAckCommand(sequence: number): string {
  const seq = Math.max(0, Math.round(finiteOr(sequence, 0)));
  return `saturn_display_ack:${seq};`;
}

/** True when the greeting advertised `spectrum_u8` among the display caps. */
export function spectrumCapsAdvertised(argText: string | null | undefined): boolean {
  if (!argText) return false;
  return String(argText)
    .split(',')
    .map((token) => token.trim().toLowerCase())
    .includes(SPECTRUM_DISPLAY_CAPS_TOKEN);
}

/**
 * Parse the bridge echo `saturn_display:0,spectrum,<fft>,<ms>;` or
 * `saturn_display:0,iq;`. Values are the bridge's effective (already clamped)
 * ones; they are re-clamped here so a malformed echo can never widen the
 * contract. Returns null for anything unrecognised.
 */
export function parseDisplayEcho(args: readonly (string | undefined)[]): DisplayEcho | null {
  const tokens = args.map((arg) => String(arg ?? '').trim().toLowerCase());
  const modeIndex = tokens.findIndex((token) => token === 'iq' || token === 'spectrum');
  if (modeIndex < 0) return null;
  if (tokens[modeIndex] === 'iq') return { mode: 'iq' };

  const fftSize = Number(tokens[modeIndex + 1]);
  const intervalMs = Number(tokens[modeIndex + 2]);
  if (!Number.isFinite(fftSize) || !Number.isFinite(intervalMs)) return null;
  return {
    mode: 'spectrum',
    fftSize: clampSpectrumFftSize(fftSize),
    intervalMs: clampSpectrumIntervalMs(intervalMs),
  };
}

function isPowerOfTwo(value: number): boolean {
  return Number.isInteger(value) && value > 0 && (value & (value - 1)) === 0;
}

/**
 * Validate a type-16 frame and return its header, or null when the frame is
 * malformed. Rejects anything outside the frozen contract so a protocol bug
 * cannot be rendered as if it were a spectrum.
 */
export function parseSpectrumRowHeader(buffer: ArrayBuffer): SpectrumRowHeader | null {
  if (buffer.byteLength < SPECTRUM_ROW_HEADER_BYTES) return null;
  const view = new DataView(buffer);
  const streamType = view.getUint32(24, true);
  if (streamType !== SPECTRUM_ROW_STREAM_TYPE) return null;
  const format = view.getUint32(8, true);
  if (format !== SPECTRUM_ROW_FORMAT_U8_DB) return null;
  const fftSize = view.getUint32(12, true);
  if (!isPowerOfTwo(fftSize) || fftSize < SPECTRUM_FFT_MIN || fftSize > SPECTRUM_FFT_MAX) return null;
  const flags = view.getUint32(16, true);
  if ((flags & SPECTRUM_ROW_FLAG_FFTSHIFTED) === 0) return null;
  const binCount = view.getUint32(20, true);
  if (binCount !== fftSize || binCount > SPECTRUM_ROW_MAX_BINS) return null;
  if (buffer.byteLength < SPECTRUM_ROW_HEADER_BYTES + binCount) return null;
  const dbOffset = view.getFloat32(44, true);
  const dbStep = view.getFloat32(48, true);
  if (!Number.isFinite(dbOffset) || !Number.isFinite(dbStep) || dbStep <= 0) return null;

  return {
    receiver: view.getUint32(0, true),
    spanHz: view.getUint32(4, true),
    format,
    fftSize,
    flags,
    binCount,
    streamType,
    channels: view.getUint32(28, true),
    sequence: view.getUint32(32, true),
    // read as two u32s to avoid a BigInt dependency; radio centers are far
    // below 2^53 so the exact integer round-trips through a double.
    centerHz: view.getUint32(36, true) + view.getUint32(40, true) * 0x1_0000_0000,
    dbOffset,
    dbStep,
    captureToEnqueueUs: view.getUint32(52, true),
    serverMs: view.getUint32(56, true),
  };
}

/** Sequence number of a frame we may still want to ack, or null. */
export function spectrumRowSequence(buffer: ArrayBuffer): number | null {
  if (buffer.byteLength < SPECTRUM_ROW_HEADER_BYTES) return null;
  if (new DataView(buffer).getUint32(24, true) !== SPECTRUM_ROW_STREAM_TYPE) return null;
  return new DataView(buffer).getUint32(32, true);
}

/** Dequantize into `target` (reused across rows) and return it. */
export function dequantizeSpectrumRow(
  buffer: ArrayBuffer,
  header: SpectrumRowHeader,
  target: Float32Array,
): Float32Array {
  const payload = new Uint8Array(buffer, SPECTRUM_ROW_HEADER_BYTES, header.binCount);
  const { dbOffset, dbStep } = header;
  for (let i = 0; i < header.binCount; i += 1) {
    target[i] = dbOffset + (payload[i] ?? 0) * dbStep;
  }
  return target;
}
