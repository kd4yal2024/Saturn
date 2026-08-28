export type PeakAssistResult = {
  bin: number;
  frequencyHz: number;
  levelDb: number;
  prominenceDb: number;
};

export type PeakAssistTrackingState = {
  candidate: PeakAssistResult | null;
  candidateFrames: number;
  locked: PeakAssistResult | null;
  missingFrames: number;
};

export function emptyPeakAssistTrackingState(): PeakAssistTrackingState {
  return { candidate: null, candidateFrames: 0, locked: null, missingFrames: 0 };
}

/**
 * Find the strongest displayed FFT peak inside the current receive passband.
 * The returned frequency is absolute and uses three-bin parabolic refinement
 * when both neighboring bins are available.
 */
export function detectPeakInPassband(
  bins: ArrayLike<number>,
  centerHz: number,
  spanHz: number,
  passbandLowHz: number,
  passbandHighHz: number,
  minimumProminenceDb = 6,
): PeakAssistResult | null {
  const length = Math.max(0, Math.floor(Number(bins?.length) || 0));
  if (length < 3 || !Number.isFinite(centerHz) || !Number.isFinite(spanHz) || spanHz <= 0) {
    return null;
  }

  const lowHz = Math.min(passbandLowHz, passbandHighHz);
  const highHz = Math.max(passbandLowHz, passbandHighHz);
  const start = Math.max(1, Math.ceil(((lowHz / spanHz) + 0.5) * length));
  const end = Math.min(length - 2, Math.floor(((highHz / spanHz) + 0.5) * length));
  if (start > end) return null;

  let peakBin = -1;
  let peakDb = Number.NEGATIVE_INFINITY;
  const finiteLevels: number[] = [];
  for (let index = start; index <= end; index += 1) {
    const value = Number(bins[index]);
    if (Number.isFinite(value)) {
      finiteLevels.push(value);
      if (value > peakDb) {
        peakDb = value;
        peakBin = index;
      }
    }
  }
  if (peakBin < 0) return null;

  finiteLevels.sort((a, b) => a - b);
  const noiseIndex = Math.max(0, Math.min(finiteLevels.length - 1, Math.floor(finiteLevels.length * 0.35)));
  const noiseFloorDb = finiteLevels[noiseIndex] ?? peakDb;
  const prominenceDb = peakDb - noiseFloorDb;
  if (prominenceDb < Math.max(0, Number(minimumProminenceDb) || 0)) return null;

  const left = Number(bins[peakBin - 1]);
  const right = Number(bins[peakBin + 1]);
  let fraction = 0;
  if (Number.isFinite(left) && Number.isFinite(right)) {
    const denominator = left - (2 * peakDb) + right;
    if (Math.abs(denominator) > 1e-9) {
      fraction = Math.max(-0.5, Math.min(0.5, 0.5 * (left - right) / denominator));
    }
  }

  const refinedBin = peakBin + fraction;
  const frequencyHz = centerHz + (((refinedBin + 0.5) / length) - 0.5) * spanHz;
  return { bin: refinedBin, frequencyHz, levelDb: peakDb, prominenceDb };
}

/**
 * Hold a peak until a replacement is present for several consecutive frames.
 * This prevents the tuning marker from chasing individual voice syllables or
 * hopping onto momentary noise spikes.
 */
export function trackPeakAssist(
  previous: PeakAssistTrackingState,
  detected: PeakAssistResult | null,
  binWidthHz: number,
): { state: PeakAssistTrackingState; peak: PeakAssistResult | null } {
  const state: PeakAssistTrackingState = {
    candidate: previous.candidate ? { ...previous.candidate } : null,
    candidateFrames: Math.max(0, previous.candidateFrames || 0),
    locked: previous.locked ? { ...previous.locked } : null,
    missingFrames: Math.max(0, previous.missingFrames || 0),
  };
  const binHz = Math.max(1, Number(binWidthHz) || 1);
  const candidateToleranceHz = Math.max(75, binHz * 2.5);
  const trackingToleranceHz = Math.max(180, binHz * 4);

  if (!detected) {
    state.candidate = null;
    state.candidateFrames = 0;
    state.missingFrames += 1;
    if (state.missingFrames > 6) state.locked = null;
    return { state, peak: state.locked };
  }

  state.missingFrames = 0;
  if (state.candidate && Math.abs(state.candidate.frequencyHz - detected.frequencyHz) <= candidateToleranceHz) {
    state.candidate = detected;
    state.candidateFrames += 1;
  } else {
    state.candidate = detected;
    state.candidateFrames = 1;
  }

  if (!state.locked) {
    if (state.candidateFrames >= 3) state.locked = detected;
    return { state, peak: state.locked };
  }

  if (Math.abs(state.locked.frequencyHz - detected.frequencyHz) <= trackingToleranceHz) {
    state.locked = {
      ...detected,
      bin: (state.locked.bin * 0.75) + (detected.bin * 0.25),
      frequencyHz: (state.locked.frequencyHz * 0.75) + (detected.frequencyHz * 0.25),
      levelDb: Math.max(detected.levelDb, (state.locked.levelDb * 0.7) + (detected.levelDb * 0.3)),
    };
  } else if (state.candidateFrames >= 5) {
    state.locked = detected;
  }

  return { state, peak: state.locked };
}
