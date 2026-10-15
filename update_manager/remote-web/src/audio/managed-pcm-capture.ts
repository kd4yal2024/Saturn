import type { ManagedCaptureFactory } from '../transport/managed-pcm';

// No resampler, ScriptProcessor fallback, retained packet tail or shared legacy
// capture. Each authorized generation owns new context, track and worklet.
export const MANAGED_PCM_WORKLET = `
class ManagedPcmCapture extends AudioWorkletProcessor {
  constructor(options) {
    super(); this.start = options.processorOptions.startFrame;
    this.buf = new Float32Array(512); this.pos = 0; this.oldest = 0;
  }
  process(inputs, outputs) {
    const mono = inputs[0]?.[0];
    if (mono) for (let i = 0; i < mono.length; i++) {
      const frame = currentFrame + i;
      if (frame < this.start) continue;
      if (!this.pos) this.oldest = frame;
      this.buf[this.pos++] = mono[i];
      if (this.pos === 512) {
        this.port.postMessage({samples: this.buf, oldestFrame: this.oldest}, [this.buf.buffer]);
        this.buf = new Float32Array(512); this.pos = 0;
      }
    }
    for (const output of outputs) for (const channel of output) channel.fill(0);
    return true;
  }
}
registerProcessor('saturn-managed-pcm', ManagedPcmCapture);
`;

export function microphoneManagedCapture(deviceId: () => string,
  level: (samples: Float32Array) => void): ManagedCaptureFactory {
  return async (token, deliver, failed) => {
    if (!globalThis.isSecureContext || !navigator.mediaDevices?.getUserMedia)
      throw new Error('Secure microphone capture required');
    const id = deviceId();
    const stream = await navigator.mediaDevices.getUserMedia({audio: {
      sampleRate: 48000, channelCount: 1, echoCancellation: false,
      noiseSuppression: false, autoGainControl: false,
      ...(id ? {deviceId: {exact: id}} : {}),
    }});
    let context: AudioContext | null = null;
    let input: MediaStreamAudioSourceNode | null = null;
    let node: AudioWorkletNode | null = null;
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      if (node) { node.port.onmessage = null; node.disconnect(); }
      input?.disconnect(); stream.getTracks().forEach(t => t.stop());
      if (context) void context.close();
    };
    try {
      context = new AudioContext({sampleRate: 48000, latencyHint: 'interactive'});
      await context.resume();
      if (context.sampleRate !== 48000 || !context.audioWorklet)
        throw new Error('Managed PCM requires 48 kHz AudioWorklet; no legacy fallback');
      const url = URL.createObjectURL(new Blob([MANAGED_PCM_WORKLET], {type: 'text/javascript'}));
      try { await context.audioWorklet.addModule(url); } finally { URL.revokeObjectURL(url); }
      const reset = performance.now(), audioAnchor = context.currentTime * 1000;
      const startFrame = Math.ceil((context.currentTime + 0.02) * 48000 / 128) * 128;
      node = new AudioWorkletNode(context, 'saturn-managed-pcm', {
        numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
        processorOptions: {startFrame},
      });
      const ctx = context;
      node.port.onmessage = e => {
        if (closed) return;
        const samples: unknown = e.data?.samples, frame: unknown = e.data?.oldestFrame;
        if (!(samples instanceof Float32Array) || typeof frame !== 'number' || !Number.isFinite(frame)) {
          failed('Malformed PCM worklet observation'); return;
        }
        // Conservative graph age, NOT physical microphone ADC or RF-output age.
        const oldest = Math.min(performance.now(), reset + frame / 48 - audioAnchor - ctx.baseLatency * 1000);
        level(samples); deliver(samples, oldest);
      };
      node.onprocessorerror = () => { if (!closed) failed('Managed PCM worklet failed'); };
      for (const track of stream.getTracks()) {
        track.onended = () => { if (!closed) failed('Microphone track ended'); };
      }
      input = context.createMediaStreamSource(stream);
      input.connect(node); node.connect(context.destination);
      return {token, historyResetAtMs: reset, close};
    } catch (error) { close(); throw error; }
  };
}
