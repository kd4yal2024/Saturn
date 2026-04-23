import { describe, expect, it } from 'vitest';
import { createRemoteController } from '../src/controller/create-controller';
import { createRemoteSession } from '../src/runtime/session';
import { TCI_FRAME_HEADER_BYTES } from '../src/transport/tci-frame';

const env = {
  protocol: 'https:',
  host: 'radio.local:8443',
  hostname: 'radio.local',
  href: 'https://radio.local:8443/remote',
};

function buildFrame(frameType: number, sampleRate: number, channels: number, floats: number[]): ArrayBuffer {
  const buffer = new ArrayBuffer(TCI_FRAME_HEADER_BYTES + floats.length * 4);
  const view = new DataView(buffer);
  view.setUint32(4, sampleRate, true);
  view.setUint32(20, floats.length, true);
  view.setUint32(24, frameType, true);
  view.setUint32(28, channels, true);
  new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, floats.length).set(floats);
  return buffer;
}

describe('runtime session', () => {
  it('routes text messages into the controller', () => {
    const controller = createRemoteController(env);
    const session = createRemoteSession(controller);
    const result = session.handleMessage('ready;dds:0,7150000;', 1000);

    expect(result.kind).toBe('text');
    expect(result.accepted).toBe(true);
    if (result.kind !== 'text') throw new Error('expected text result');
    expect(result.ready).toBe(true);
    expect(controller.getState().radio.dds).toBe(7150000);
  });

  it('routes binary messages into the controller', () => {
    const controller = createRemoteController(env);
    const session = createRemoteSession(controller);
    const result = session.handleMessage(buildFrame(0, 192000, 2, [1, 2, 3, 4]), 3000);

    expect(result.kind).toBe('binary');
    expect(result.accepted).toBe(true);
    if (result.kind !== 'binary') throw new Error('expected binary result');
    expect(result.frameKind).toBe('iq');
    expect(controller.getState().transport.iqPackets).toHaveLength(1);
  });

  it('ignores unsupported message payloads', () => {
    const controller = createRemoteController(env);
    const session = createRemoteSession(controller);
    const result = session.handleMessage({ nope: true }, 1000);

    expect(result.kind).toBe('ignored');
    expect(result.accepted).toBe(false);
  });
});
