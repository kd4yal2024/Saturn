export interface FftJob {
  kind: 'fft'; id: number; generation: number; fftSize: number; floatCount: number;
  input: ArrayBuffer; output: ArrayBuffer;
}
export interface FftReply extends Omit<FftJob, 'kind'> {
  kind: 'result'; fftMs: number; error?: string;
  workspaceAllocations?: number; workspaceBytes?: number;
}
export interface DisplayRequest {
  key: string; sequence: number; fftSize: number; floatCount: number;
  newestIngressMs: number; oldestIngressMs: number; intervalMs: number;
  /** Write synchronously from the retained source into the producer-owned slot.
   * Return the actual float count. Never transfer a source/capture buffer. */
  write(destination: Float32Array): number;
}
export interface CompletedSpectrum {
  id: number; generation: number; sequence: number; fftSize: number;
  newestIngressMs: number; oldestIngressMs: number; completedAtMs: number;
  /** Borrowed, main-owned until the next producer callback/reset. Synchronous
   * rendering/capture must copy any data it wants to retain. */
  bins: Float32Array;
}
