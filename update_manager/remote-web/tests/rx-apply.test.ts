import { describe, expect, it } from 'vitest';
import { appendIqPacket, applyAudioFrame, applyIqFrame, buildRenderIqWindow, fftSizeForSamples } from '../src/transport/rx-apply';
import type { AudioFrame, IqFrame } from '../src/transport/rx-frame';
import type { RxTransportState } from '../src/transport/rx-state';

function createState(): RxTransportState {
  return {
    sampleRate: 192000,
    displayZoom: 1,
    iqPackets: [],
    maxFftSize: 262144,
    baseFftSize: 4096,
    packetHistoryBase: 48,
    packetHistoryMax: 1024,
    moxRequested: false,
    txEnabled: false,
    audioStreaming: true,
    audioSampleRate: 48000,
    audioFramesPlayed: 0,
    lastFrameAt: 0,
    frameCounter: 0,
    displayCaption: '',
  };
}

describe('rx transport apply', () => {
  it('computes fft sizes and appends packet history', () => {
    expect(fftSizeForSamples(1000, 1, 4096, 262144)).toBe(512);
    const state = appendIqPacket(createState(), new Float32Array([1, 2, 3, 4]));
    expect(state.iqPackets).toHaveLength(1);
  });

  it('builds a render window from queued iq packets', () => {
    let state = createState();
    state = appendIqPacket(state, new Float32Array([1, 2]));
    state = appendIqPacket(state, new Float32Array([3, 4]));
    expect(Array.from(buildRenderIqWindow(state) || [])).toEqual([1, 2, 3, 4]);
  });

  it('applies iq frames and reports sample-rate changes', () => {
    const frame: IqFrame = {
      kind: 'iq',
      header: { sampleRate: 384000, floatCount: 4, frameType: 0, channels: 2 },
      iq: new Float32Array([1, 2, 3, 4]),
      samplePairs: 2,
    };
    const result = applyIqFrame(createState(), frame, 3000);
    expect(result.accepted).toBe(true);
    expect(result.sampleRateChanged).toBe(true);
    expect(result.state.sampleRate).toBe(384000);
    expect(result.state.frameCounter).toBe(1);
    expect(result.receivedIqLogMessage).toContain('Received IQ frame');
  });

  it('drops iq frames while transmit is active', () => {
    const state = createState();
    state.txEnabled = true;
    const frame: IqFrame = {
      kind: 'iq',
      header: { sampleRate: 192000, floatCount: 4, frameType: 0, channels: 2 },
      iq: new Float32Array([1, 2, 3, 4]),
      samplePairs: 2,
    };
    const result = applyIqFrame(state, frame, 100);
    expect(result.accepted).toBe(false);
  });

  it('applies audio frames only while rx audio is active', () => {
    const frame: AudioFrame = {
      kind: 'audio',
      header: { sampleRate: 48000, floatCount: 4, frameType: 1, channels: 2 },
      frames: 2,
      left: new Float32Array([0.1, 0.2]),
      right: new Float32Array([0.1, 0.2]),
      mirroredMono: true,
    };
    const accepted = applyAudioFrame(createState(), frame);
    expect(accepted.accepted).toBe(true);
    expect(accepted.state.audioFramesPlayed).toBe(1);

    const suspended = createState();
    suspended.audioStreaming = false;
    expect(applyAudioFrame(suspended, frame).accepted).toBe(false);
  });
});
