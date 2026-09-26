import { describe, expect, it } from 'vitest';
import {
  copyRxOpusAudio, createRxAudioCodecSession, parseRxOpusPacket, probeRxOpusDecoder,
  rxOpusBacklogAction, rxOpusMalformedAction,
} from '../src/audio/rx-opus';
import { decodeRxFrame } from '../src/transport/rx-frame';

function opusFrame(payload: number[], sampleRate = 48_000, channels = 1, sequence = 7): ArrayBuffer {
  const buffer = new ArrayBuffer(64 + payload.length);
  const view = new DataView(buffer);
  view.setUint32(4, sampleRate, true);
  view.setUint32(8, 20, true);
  view.setUint32(12, 0, true);
  view.setUint32(16, 960, true);
  view.setUint32(20, payload.length, true);
  view.setUint32(24, 17, true);
  view.setUint32(28, channels, true);
  view.setUint32(32, sequence, true);
  new Uint8Array(buffer, 64).set(payload);
  return buffer;
}

describe('RX Opus transport', () => {
  it('parses a type 17 byte payload and never treats it as IQ floats', () => {
    const buffer = opusFrame([0x48, 0x12, 0x34], 48_000, 2, 25);
    expect(parseRxOpusPacket(buffer)).toMatchObject({
      sampleRate: 48_000, channels: 2, sequence: 25, frames: 960,
      payload: new Uint8Array([0x48, 0x12, 0x34]),
    });
    expect(decodeRxFrame(buffer)).toBeNull();
  });

  it('rejects malformed and inconsistent headers without reading beyond the buffer', () => {
    expect(parseRxOpusPacket(new ArrayBuffer(63))).toBeNull();
    expect(parseRxOpusPacket(opusFrame([]))).toBeNull();
    expect(parseRxOpusPacket(opusFrame([1], 12_000))).toBeNull();
    expect(parseRxOpusPacket(opusFrame([1], 48_000, 3))).toBeNull();
    const wrongDuration = opusFrame([1]);
    new DataView(wrongDuration).setUint32(8, 40, true);
    expect(parseRxOpusPacket(wrongDuration)).toBeNull();
    const fecFlag = opusFrame([1]);
    new DataView(fecFlag).setUint32(12, 1, true);
    expect(parseRxOpusPacket(fecFlag)).toBeNull();
    const wrongSampleCount = opusFrame([1]);
    new DataView(wrongSampleCount).setUint32(16, 480, true);
    expect(parseRxOpusPacket(wrongSampleCount)).toBeNull();
    const truncated = opusFrame([1, 2]);
    new DataView(truncated).setUint32(20, 3, true);
    expect(parseRxOpusPacket(truncated)).toBeNull();
    const trailing = opusFrame([1, 2]);
    new DataView(trailing).setUint32(20, 1, true);
    expect(parseRxOpusPacket(trailing)).toBeNull();
  });

  it('requires WebCodecs Opus support for both WAN mono and LAN stereo', async () => {
    expect(await probeRxOpusDecoder(undefined)).toBe(false);
    const calls: number[] = [];
    expect(await probeRxOpusDecoder({
      async isConfigSupported(config) {
        calls.push(config.numberOfChannels);
        return {supported: config.numberOfChannels === 1};
      },
    })).toBe(false);
    expect(calls).toEqual([1, 2]);
    expect(await probeRxOpusDecoder({
      async isConfigSupported() { throw new Error('unsupported'); },
    })).toBe(false);
  });

  it('switches only after an accepted echo, holds PCM after a decoder fault, and resets on reconnect', () => {
    const session = createRxAudioCodecSession();
    expect(session.accepted).toBe('pcm');
    expect(session.acceptEcho('opus')).toBe(false);
    expect(session.requestOpus()).toBe(true);
    expect(session.accepted).toBe('pcm');
    expect(session.acceptEcho('opus')).toBe(true);
    expect(session.accepted).toBe('opus');
    expect(session.fallback()).toBe(true);
    expect(session.accepted).toBe('pcm');
    expect(session.acceptEcho('opus')).toBe(false);
    expect(session.requestOpus()).toBe(false);
    session.reset();
    expect(session.failed).toBe(false);
    expect(session.requestOpus()).toBe(true);
    expect(session.acceptEcho('opus')).toBe(true);
  });

  it('copies decoded mono into both sink channels and preserves stereo', () => {
    const mono = copyRxOpusAudio({
      sampleRate: 48_000, numberOfChannels: 1, numberOfFrames: 2,
      copyTo(destination) { destination.set([0.25, -0.5]); },
    });
    expect(Array.from(mono?.left || [])).toEqual([0.25, -0.5]);
    expect(Array.from(mono?.right || [])).toEqual([0.25, -0.5]);
    const stereo = copyRxOpusAudio({
      sampleRate: 48_000, numberOfChannels: 2, numberOfFrames: 2,
      copyTo(destination, options) { destination.set(options.planeIndex === 0 ? [0.1, 0.2] : [0.3, 0.4]); },
    });
    expect(stereo?.right[0]).toBeCloseTo(0.3);
    expect(stereo?.right[1]).toBeCloseTo(0.4);
  });

  it('treats one corrupt packet as a dropped frame and a sustained run as a contract mismatch', () => {
    // Skip-and-resume: a corrupt packet costs one 20 ms frame, so it is dropped.
    expect(rxOpusMalformedAction(1)).toBe('drop');
    expect(rxOpusMalformedAction(4)).toBe('drop');
    // A run means the wire format itself is not what this browser expects.
    expect(rxOpusMalformedAction(5)).toBe('fallback');
    expect(rxOpusMalformedAction(6)).toBe('fallback');
    expect(rxOpusMalformedAction(2, 2)).toBe('fallback');
  });

  it('resyncs a stalled decoder instead of abandoning Opus', () => {
    expect(rxOpusBacklogAction(0)).toBe('none');
    expect(rxOpusBacklogAction(32)).toBe('none');
    expect(rxOpusBacklogAction(33)).toBe('resync');
    expect(rxOpusBacklogAction(4, 3)).toBe('resync');
  });
});
