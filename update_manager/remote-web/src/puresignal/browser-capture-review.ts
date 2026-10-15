// Offline only. Not imported by the installed remote client.
// Real MediaStream -> AudioWorklet capture of a deterministic recorded source.
import type { CaptureFactory } from '../transport/tx-generation-review';

export const REVIEW_CAPTURE_WORKLET = `
class ReviewCapture extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const p = options.processorOptions;
    this.start = p.startFrame; this.end = p.endFrame; this.size = p.blockSize;
    this.buf = new Float32Array(this.size); this.pos = 0; this.oldest = 0;
  }
  process(inputs, outputs) {
    const mono = inputs[0]?.[0];
    if (mono) for (let i = 0; i < mono.length; i++) {
      const frame = currentFrame + i;
      if (frame < this.start || frame >= this.end) continue;
      if (!this.pos) this.oldest = frame;
      this.buf[this.pos++] = mono[i];
      if (this.pos === this.size) {
        this.port.postMessage({samples: this.buf, oldestFrame: this.oldest}, [this.buf.buffer]);
        this.buf = new Float32Array(this.size); this.pos = 0;
      }
    }
    for (const out of outputs) for (const channel of out) channel.fill(0);
    if (currentFrame >= this.end) {
      this.port.postMessage({done: true, tailSamples: this.pos});
      return false;
    }
    return true;
  }
}
registerProcessor('ps-recorded-capture-review', ReviewCapture);
`;

export interface CaptureAudit {
  resetAtMs: number; sampleRate: number; inputSamples: number;
  tailSamples: number; blocks: number; workletToMainMs: number[];
  observations: number[]; done: Promise<void>;
}

export function recordedCapture(samples: Float32Array, blockSize: number,
  audit: (value: CaptureAudit) => void): CaptureFactory {
  return async (token, deliver) => {
    const ctx = new AudioContext({sampleRate: 48000, latencyHint: 'interactive'});
    let closed = false;
    let finish!: () => void;
    const done = new Promise<void>(resolve => { finish = resolve; });
    const close = () => {
      if (closed) return;
      closed = true; finish(); void ctx.close();
    };
    try {
      await ctx.resume();
      if (ctx.sampleRate !== 48000) throw new Error('capture_rate_mismatch');
      const url = URL.createObjectURL(new Blob([REVIEW_CAPTURE_WORKLET], {type: 'text/javascript'}));
      try { await ctx.audioWorklet.addModule(url); } finally { URL.revokeObjectURL(url); }
      const buffer = ctx.createBuffer(1, samples.length, 48000);
      buffer.copyToChannel(new Float32Array(samples), 0);
      const source = ctx.createBufferSource(); source.buffer = buffer;
      // Same capture API as the microphone path. Recorded source replaces only
      // the physical microphone, not AudioWorklet or WebCodecs with a shim.
      const stream = ctx.createMediaStreamDestination(); source.connect(stream);
      const input = ctx.createMediaStreamSource(stream.stream);
      const resetAtMs = performance.now();
      const anchorAudioMs = ctx.currentTime * 1000;
      const anchorMainMs = resetAtMs;
      const startFrame = Math.ceil((ctx.currentTime + 0.1) * 48000 / 128) * 128;
      const node = new AudioWorkletNode(ctx, 'ps-recorded-capture-review', {
        numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
        processorOptions: {startFrame, endFrame: startFrame + samples.length, blockSize},
      });
      const state: CaptureAudit = {resetAtMs, sampleRate: ctx.sampleRate,
        inputSamples: samples.length, blocks: 0, tailSamples: 0,
        workletToMainMs: [], observations: [], done};
      node.port.onmessage = e => {
        if (closed) return;
        if (e.data.done) { state.tailSamples = e.data.tailSamples; finish(); return; }
        const oldestAudioMs = Number(e.data.oldestFrame) / 48;
        // Conservative bridge-local graph age: do not use receipt as the
        // observation. Account for graph buffering, never timestamp in future.
        const oldest = Math.min(performance.now(), anchorMainMs + oldestAudioMs
          - anchorAudioMs - ctx.baseLatency * 1000);
        state.blocks++;
        state.workletToMainMs.push(performance.now() - oldest);
        state.observations.push(oldest);
        deliver(e.data.samples, oldest);
      };
      input.connect(node); node.connect(ctx.destination);
      audit(state);
      source.start(startFrame / 48000);
      return {token, historyResetAtMs: resetAtMs, close};
    } catch (error) { close(); throw error; }
  };
}
