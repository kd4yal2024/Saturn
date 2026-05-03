export type AnrPreset = {
  rxAnrTaps: number;
  rxAnrDelay: number;
  rxAnrGain: number;
  rxAnrLeakage: number;
};

export type AnfPreset = {
  rxAnfTaps: number;
  rxAnfDelay: number;
  rxAnfGain: number;
  rxAnfLeakage: number;
};

export function anrPreset(name: 'wide' | 'thetis'): AnrPreset {
  if (name === 'wide') {
    return { rxAnrTaps: 96, rxAnrDelay: 24, rxAnrGain: 0.00035, rxAnrLeakage: 0.00003 };
  }
  return { rxAnrTaps: 64, rxAnrDelay: 16, rxAnrGain: 0.0002, rxAnrLeakage: 0.00005 };
}

export function anfPreset(name: 'sharp' | 'default'): AnfPreset {
  if (name === 'sharp') {
    return { rxAnfTaps: 96, rxAnfDelay: 12, rxAnfGain: 0.00018, rxAnfLeakage: 0.00005 };
  }
  return { rxAnfTaps: 64, rxAnfDelay: 16, rxAnfGain: 0.00012, rxAnfLeakage: 0.00008 };
}
