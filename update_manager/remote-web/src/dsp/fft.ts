export class FftProcessor {
  readonly size: number;
  private window: Float32Array;
  private bitReverse: Uint32Array;
  private cosTable: Float32Array;
  private sinTable: Float32Array;
  private scratchRe: Float32Array;
  private scratchIm: Float32Array;
  private prevBins: Float32Array;

  constructor(size: number) {
    this.size = size;
    this.window = new Float32Array(size);
    this.bitReverse = new Uint32Array(size);
    this.cosTable = new Float32Array(size / 2);
    this.sinTable = new Float32Array(size / 2);
    this.scratchRe = new Float32Array(size);
    this.scratchIm = new Float32Array(size);
    this.prevBins = new Float32Array(size);

    for (let i = 0; i < size; i += 1) {
      this.window[i] = 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (size - 1));
      let j = 0;
      for (let bit = 0; (1 << bit) < size; bit += 1) {
        j = (j << 1) | ((i >> bit) & 1);
      }
      this.bitReverse[i] = j;
    }
    for (let i = 0; i < size / 2; i += 1) {
      const angle = (-2 * Math.PI * i) / size;
      this.cosTable[i] = Math.cos(angle);
      this.sinTable[i] = Math.sin(angle);
    }
  }

  transform(iqInterleaved: Float32Array): Float32Array {
    const samplePairs = Math.floor(iqInterleaved.length / 2);
    if (samplePairs < 1) {
      return new Float32Array(this.size);
    }
    const stride = Math.max(1, Math.floor(samplePairs / this.size));

    for (let i = 0; i < this.size; i += 1) {
      const sourcePair = Math.min(samplePairs - 1, i * stride);
      const sourceOffset = sourcePair * 2;
      const w = this.window[i] ?? 0;
      const target = this.bitReverse[i] ?? 0;
      this.scratchRe[target] = (iqInterleaved[sourceOffset] ?? 0) * w;
      this.scratchIm[target] = (iqInterleaved[sourceOffset + 1] ?? 0) * w;
    }

    for (let span = 2; span <= this.size; span <<= 1) {
      const half = span >> 1;
      const tableStep = this.size / span;
      for (let start = 0; start < this.size; start += span) {
        for (let i = 0; i < half; i += 1) {
          const twiddleIndex = i * tableStep;
          const evenIndex = start + i;
          const oddIndex = evenIndex + half;
          const oddRe = this.scratchRe[oddIndex] ?? 0;
          const oddIm = this.scratchIm[oddIndex] ?? 0;
          const twiddleRe = this.cosTable[twiddleIndex] ?? 1;
          const twiddleIm = this.sinTable[twiddleIndex] ?? 0;
          const rotRe = oddRe * twiddleRe - oddIm * twiddleIm;
          const rotIm = oddRe * twiddleIm + oddIm * twiddleRe;
          const evenRe = this.scratchRe[evenIndex] ?? 0;
          const evenIm = this.scratchIm[evenIndex] ?? 0;
          this.scratchRe[oddIndex] = evenRe - rotRe;
          this.scratchIm[oddIndex] = evenIm - rotIm;
          this.scratchRe[evenIndex] = evenRe + rotRe;
          this.scratchIm[evenIndex] = evenIm + rotIm;
        }
      }
    }

    const shifted = new Float32Array(this.size);
    const half = this.size / 2;
    for (let i = 0; i < this.size; i += 1) {
      const shiftedIndex = (i + half) % this.size;
      const re = this.scratchRe[shiftedIndex] ?? 0;
      const im = this.scratchIm[shiftedIndex] ?? 0;
      const magnitude = Math.hypot(re, im) / this.size;
      const db = 20 * Math.log10(magnitude + 1e-8);
      const smoothed = (this.prevBins[i] ?? 0) * 0.82 + db * 0.18;
      this.prevBins[i] = smoothed;
      shifted[i] = smoothed;
    }
    return shifted;
  }

  resetSmoothing(): void {
    this.prevBins.fill(0);
  }
}
