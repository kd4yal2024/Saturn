import {afterEach, describe, expect, it, vi} from 'vitest';
import {microphoneManagedCapture, type MicrophoneStartupTiming} from '../src/audio/managed-pcm-capture';

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
describe('managed microphone startup observations', () => {
  function fixture() {
    let now = 0;
    let node: {port: {onmessage: ((e: {data: unknown}) => void) | null}};
    const stop = vi.fn(), disconnect = vi.fn(), close = vi.fn();
    vi.stubGlobal('performance', {now: () => now});
    vi.stubGlobal('isSecureContext', true);
    vi.stubGlobal('navigator', {mediaDevices: {getUserMedia: vi.fn(async () => {
      now = 600; return {getTracks: () => [{stop}]};
    })}});
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:managed-test');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
    vi.stubGlobal('AudioContext', class {
      sampleRate = 48000; currentTime = 1; baseLatency = 0.004;
      destination = {};
      audioWorklet = {addModule: async () => { now += 150; }};
      async resume() { now += 200; }
      createMediaStreamSource() { return {connect: vi.fn(), disconnect}; }
      close = close;
    });
    vi.stubGlobal('AudioWorkletNode', class {
      port = {onmessage: null as ((e: {data: unknown}) => void) | null};
      connect = vi.fn(); disconnect = disconnect;
      constructor() { node = this; }
    });
    return {stop, close, frame: (samples: unknown = new Float32Array(512)) => {
      now += 30;
      node.port.onmessage?.({data: {samples, oldestFrame: 48000}});
    }};
  }
  it('separates device acquisition, graph setup and first delivery once per generation', async () => {
    const f = fixture(), timing: MicrophoneStartupTiming[] = [], deliver = vi.fn();
    const capture = await microphoneManagedCapture(() => '', vi.fn(), v => timing.push(v))('fresh', deliver, vi.fn());
    expect(timing).toEqual([]);
    f.frame(); f.frame();
    expect(timing).toEqual([{acquisitionMs: 600, graphReadyMs: 950, firstFrameMs: 980, baseLatencyMs: 4}]);
    expect(deliver).toHaveBeenCalledTimes(2);
    capture.close(); f.frame();
    expect(deliver).toHaveBeenCalledTimes(2);
    expect(f.stop).toHaveBeenCalledOnce();
  });
  it('does not report malformed or cancelled capture as usable startup', async () => {
    const f = fixture(), timing = vi.fn(), failed = vi.fn();
    const capture = await microphoneManagedCapture(() => '', vi.fn(), timing)('fresh', vi.fn(), failed);
    f.frame(null);
    expect(failed).toHaveBeenCalledWith('Malformed PCM worklet observation');
    expect(timing).not.toHaveBeenCalled();
    capture.close(); f.frame();
    expect(timing).not.toHaveBeenCalled();
  });
});
