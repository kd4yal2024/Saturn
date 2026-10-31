import type { ManagedCaptureFactory } from '../transport/managed-pcm';
import { createTimingProbePcm } from '../puresignal/timing-probe';

/** Source-generated 48 kHz PCM, not microphone audio. Fresh state per generation.
 * PCM stimuli stop on a missed sample clock. Native-generator keepalives send
 * only newly generated zeros, never catch up or replay retained audio. Their
 * 500 ms liveness bound matches ManagedPcmSession and the host source watchdog.
 * No socket, PTT, generation grant or RF permission is created here. */
export function twoToneManagedCapture(clock: {
  now(): number;
  later(callback: () => void, delay: number): ReturnType<typeof setTimeout>;
  cancel(timer: ReturnType<typeof setTimeout>): void;
} = { now: () => performance.now(), later: (callback, delay) => setTimeout(callback, delay), cancel: timer => clearTimeout(timer) },
  stimulus: () => 'two-tone' | 'am-carrier' | 'timing' | 'keepalive' = () => 'two-tone'): ManagedCaptureFactory {
  // Prepared once, before requesting RF, not in the timed delivery callback.
  // Repeat period is explicit in the negotiated waveform identity/capture.
  let timing: Float32Array | undefined;
  if (stimulus() === 'timing') timing = createTimingProbePcm(1000).samples;
  return async (token, deliver, failed) => {
    // Snapshot once per authorized generation. A UI mode change may not turn
    // retained callbacks into a different waveform halfway through an action.
    const waveform = stimulus();
    if (waveform !== 'two-tone' && waveform !== 'am-carrier' && waveform !== 'timing' && waveform !== 'keepalive') throw new Error('Unsupported diagnostic waveform');
    if (waveform === 'timing' && !timing) throw new Error('Timing waveform was not prepared before authorization');
    const reset = clock.now();
    let next = reset, lastDelivery = reset, sample = 0, closed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const close = () => { closed = true; if (timer !== undefined) clock.cancel(timer); };
    const pump = () => {
      if (closed) return;
      const now = clock.now();
      if (waveform === 'keepalive' ? now - lastDelivery > 500 : now - next > 50) {
        close(); failed(waveform === 'keepalive' ? 'Native generator keepalive expired; no catch-up burst'
          : 'Two-tone scheduling deadline; no catch-up burst'); return;
      }
      const samples = new Float32Array(512);
      if (waveform === 'two-tone') {
        for (let i = 0; i < samples.length; i++, sample++)
          samples[i] = .2 * (Math.sin(2 * Math.PI * 700 * sample / 48000) + Math.sin(2 * Math.PI * 1900 * sample / 48000));
      } else if (waveform === 'timing') {
        for (let i = 0; i < samples.length; i++, sample++) samples[i] = timing![sample % timing!.length]!;
      }
      // AM carrier is produced by WDSP, not by an injected DC audio value.
      // Explicit zero PCM maintains genuine ingress liveness during dead-key.
      deliver(samples, now);
      lastDelivery = now;
      next = waveform === 'keepalive' ? now + 512 / 48 : next + 512 / 48;
      if (!closed) timer = clock.later(pump, Math.max(1, next - clock.now()));
    };
    // Acquire returns before first delivery, so the session can install the
    // matching token/capture. Even an already-queued callback rechecks close.
    timer = clock.later(pump, 1);
    return { token, historyResetAtMs: reset, close };
  };
}
