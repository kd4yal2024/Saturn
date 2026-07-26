/**
 * Pure display math helpers for spectrum/waterfall rendering.
 * All functions are stateless — state values are passed as parameters.
 */

/** Visible Hz span given sample rate and zoom level. */
export function displaySpanHz(sampleRate: number, displayZoom: number): number {
  return Math.max(1000, sampleRate / Math.max(1, displayZoom));
}

/** Percent position (0–100) of an offset from center on the display. */
export function displayPercentForOffsetHz(offsetHz: number, spanHz: number): number {
  if (spanHz <= 0) return 50;
  return 50 + (offsetHz / spanHz) * 100;
}

/** Passband start/end in signed Hz from UI filter cuts. */
export { signedPassbandFromUiCuts as displayPassband } from '../radio/passband';

/**
 * Extract the visible portion of FFT bins for the current zoom level,
 * reversing bin order for display (low-freq left → high-freq right).
 */
export function visibleBinsForDisplay(
  bins: Float32Array | null,
  displayZoom: number,
): Float32Array | null {
  if (!bins || bins.length === 0) return bins;
  const visibleCount = Math.max(128, Math.floor(bins.length / Math.max(1, displayZoom)));
  const source =
    visibleCount >= bins.length
      ? bins
      : bins.subarray(
          Math.floor((bins.length - visibleCount) / 2),
          Math.floor((bins.length - visibleCount) / 2) + visibleCount,
        );
  const oriented = new Float32Array(source.length);
  for (let i = 0; i < source.length; i += 1) {
    oriented[i] = source[source.length - 1 - i] ?? 0;
  }
  return oriented;
}

/**
 * Shift bins left or right, filling empty space with `fillValue`.
 * Positive shiftBins → shift right, negative → shift left.
 */
export function shiftBinsHorizontally(
  bins: Float32Array | null,
  shiftBins: number,
  fillValue: number,
): Float32Array | null {
  if (!bins || bins.length === 0 || shiftBins === 0) return bins;
  const shifted = new Float32Array(bins.length);
  shifted.fill(fillValue);
  if (Math.abs(shiftBins) >= bins.length) return shifted;
  if (shiftBins > 0) {
    shifted.set(bins.subarray(0, bins.length - shiftBins), shiftBins);
  } else {
    shifted.set(bins.subarray(-shiftBins), 0);
  }
  return shifted;
}

/**
 * Apply a lightweight spatial moving-average to the displayed spectrum trace.
 * This is intentionally separate from temporal spectrum averaging so smoothing
 * does not add latency to tuning or waterfall updates.
 */
export function smoothSpectrumTrace(bins: Float32Array, amount: number): Float32Array {
  const strength = Math.max(0, Math.min(100, Number.isFinite(amount) ? amount : 0)) / 100;
  if (strength <= 0 || bins.length < 3) return bins;

  const radius = Math.max(1, Math.min(6, Math.ceil(strength * 6)));
  const smoothed = new Float32Array(bins.length);
  let sum = 0;
  let start = 0;
  let end = Math.min(bins.length - 1, radius);

  for (let i = start; i <= end; i += 1) sum += bins[i] ?? 0;
  for (let i = 0; i < bins.length; i += 1) {
    while (start < Math.max(0, i - radius)) {
      sum -= bins[start] ?? 0;
      start += 1;
    }
    const targetEnd = Math.min(bins.length - 1, i + radius);
    while (end < targetEnd) {
      end += 1;
      sum += bins[end] ?? 0;
    }
    const average = sum / Math.max(1, end - start + 1);
    smoothed[i] = (bins[i] ?? 0) * (1 - strength) + average * strength;
  }
  return smoothed;
}

export type BandEdgeMarker = { hz: number; label: string };

/**
 * Return band-edge markers that fall within the visible display span.
 */
export function bandEdgesInView(
  centerHz: number,
  spanHz: number,
  bandEdges: ReadonlyArray<{ start: number; end: number; label: string }>,
): BandEdgeMarker[] {
  const halfSpan = spanHz / 2;
  const low = centerHz - halfSpan;
  const high = centerHz + halfSpan;
  return bandEdges.flatMap((band) => {
    const markers: BandEdgeMarker[] = [];
    if (band.start >= low && band.start <= high) {
      markers.push({ hz: band.start, label: `${band.label} lo` });
    }
    if (band.end >= low && band.end <= high) {
      markers.push({ hz: band.end, label: `${band.label} hi` });
    }
    return markers;
  });
}

/**
 * Compute auto-range floor/ceiling from bin data using exponential smoothing.
 */
export function autoRangeFromBins(
  bins: Float32Array,
  currentFloor: number,
  currentCeiling: number,
  alpha: number,
): { floor: number; ceiling: number } {
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (let i = 0; i < bins.length; i += 1) {
    const value = bins[i];
    if (value === undefined) continue;
    if (value < min) min = value;
    if (value > max) max = value;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) {
    return { floor: currentFloor, ceiling: currentCeiling };
  }
  const span = Math.max(36, max - min + 12);
  const targetFloor = min - 6;
  const targetCeiling = targetFloor + span;
  return {
    floor: currentFloor * (1 - alpha) + targetFloor * alpha,
    ceiling: currentCeiling * (1 - alpha) + targetCeiling * alpha,
  };
}
