import { FftProcessor } from '../dsp/fft';
import type { FftJob, FftReply } from './fft-protocol';
const port = globalThis as unknown as {
  onmessage: (event: MessageEvent<FftJob>) => void;
  postMessage(reply: FftReply, transfer: ArrayBuffer[]): void;
};
let workspaceAllocations = 0, workspaceBytes = 0;
let fft: FftProcessor | null = null;
port.onmessage = ({data: job}) => {
  const start = performance.now();
  let error: string | undefined;
  try {
    if (job.kind !== 'fft' || !Number.isInteger(job.fftSize) || job.fftSize < 32 || job.fftSize > 16384
      || (job.fftSize & (job.fftSize - 1)) !== 0 || job.floatCount < 2 * job.fftSize
      || job.floatCount > job.input.byteLength / 4 || job.output.byteLength < 4 * job.fftSize) throw Error('Invalid display FFT job');
    if (fft?.size !== job.fftSize) {
      fft = new FftProcessor(job.fftSize);
      workspaceAllocations += 6; workspaceBytes += job.fftSize * 20;
    }
    fft.transformInto(new Float32Array(job.input, 0, job.floatCount), new Float32Array(job.output, 0, job.fftSize));
  } catch (failure) { error = String(failure); }
  // Both backing stores return, including on a numerical/protocol error. The
  // worker owns neither after this transfer; it keeps only FFT scratch arrays.
  port.postMessage({ ...job, kind: 'result', workspaceAllocations, workspaceBytes, fftMs: performance.now() - start, ...(error ? {error} : {}) }, [job.input, job.output]);
};
