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
    const stream = () => ({getTracks: () => [{stop, readyState: 'live'}], clone: () => stream()}) as unknown as MediaStream;
    const getUserMedia = vi.fn(async () => { now += 600; return stream(); });
    vi.stubGlobal('navigator', {mediaDevices: {getUserMedia}});
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
    return {stop, close, getUserMedia, stream, frame: (samples: unknown = new Float32Array(512)) => {
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
  it('prepares the device without delivering PCM, then creates fresh graphs without reopening it', async () => {
    const f = fixture(), deliver = vi.fn(), timings: MicrophoneStartupTiming[] = [];
    const factory = microphoneManagedCapture(() => '', vi.fn(), t => timings.push(t));
    await factory.prepare!();
    expect(f.getUserMedia).toHaveBeenCalledOnce(); expect(deliver).not.toHaveBeenCalled();
    const first = await factory('first', deliver, vi.fn());
    f.frame(); first.close();
    const second = await factory('second', deliver, vi.fn());
    f.frame(); second.close();
    expect(f.getUserMedia).toHaveBeenCalledOnce();
    expect(timings.map(t => t.acquisitionMs)).toEqual([0, 0]);
    expect(first.historyResetAtMs).toBeLessThan(second.historyResetAtMs);
    expect(deliver).toHaveBeenCalledTimes(2);
    factory.dispose!(); expect(f.stop).toHaveBeenCalledTimes(3);
  });
  it('cancels a generation waiting for device acquisition while retaining preparation for the next arm', async () => {
    const f = fixture(); let resolve!: (s: MediaStream) => void;
    f.getUserMedia.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
    const factory = microphoneManagedCapture(() => '', vi.fn());
    const old = factory('old', vi.fn(), vi.fn());
    factory.cancel!(); resolve(f.stream());
    await expect(old).rejects.toThrow('Microphone capture cancelled');
    const fresh = await factory('fresh', vi.fn(), vi.fn());
    expect(f.getUserMedia).toHaveBeenCalledOnce();
    fresh.close(); factory.dispose!();
  });
  it('discards late device acquisition after disconnect instead of warming another session', async () => {
    const f = fixture(); let resolve!: (s: MediaStream) => void;
    f.getUserMedia.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
    const factory = microphoneManagedCapture(() => '', vi.fn());
    const opening = factory.prepare!(); factory.dispose!(); resolve(f.stream());
    await expect(opening).rejects.toThrow('Microphone preparation cancelled');
    expect(f.stop).toHaveBeenCalledOnce();
    await factory.prepare!(); expect(f.getUserMedia).toHaveBeenCalledTimes(2);
    factory.dispose!();
  });
  it('cancels graph setup before a delayed context resume and prevents stale delivery', async () => {
    const f = fixture(); let resume!: () => void;
    vi.stubGlobal('AudioContext', class {
      async resume() { await new Promise<void>(r => { resume = r; }); }
      close = f.close;
    });
    const factory = microphoneManagedCapture(() => '', vi.fn()), deliver = vi.fn();
    const opening = factory('old', deliver, vi.fn());
    for (let i=0; i<8; i++) await Promise.resolve();
    factory.cancel!(); resume();
    await expect(opening).rejects.toThrow('Microphone capture cancelled');
    expect(deliver).not.toHaveBeenCalled(); expect(f.close).toHaveBeenCalledOnce();
    factory.dispose!();
  });

  it('replaces the prepared device and gives the new generation a valid capture revision', async () => {
    const f = fixture(); let id = 'first';
    const factory = microphoneManagedCapture(() => id, vi.fn());
    await factory.prepare!(); id = 'second'; await factory.prepare!();
    expect(f.getUserMedia).toHaveBeenCalledTimes(2); expect(f.stop).toHaveBeenCalledOnce();
    const capture = await factory('new-device', vi.fn(), vi.fn());
    expect(capture.token).toBe('new-device'); capture.close(); factory.dispose!();
  });

});
