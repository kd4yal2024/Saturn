import { describe, expect, it } from 'vitest';
import { createRemoteController } from '../src/controller/create-controller';
import { createRemoteSession, type RealtimeSocketLike } from '../src/runtime/session';
import { TCI_FRAME_HEADER_BYTES, TciStreamType } from '../src/transport/tci-frame';

const env = {
  protocol: 'https:',
  host: 'radio.local:8443',
  hostname: 'radio.local',
  href: 'https://radio.local:8443/remote-next',
};

class FakeBridgeSocket implements RealtimeSocketLike {
  private readonly listeners = new Map<string, Array<(event: any) => void>>();

  addEventListener(type: 'message' | 'open' | 'close' | 'error', listener: (event: any) => void): void {
    const list = this.listeners.get(type) ?? [];
    list.push(listener);
    this.listeners.set(type, list);
  }

  close(): void {
    this.dispatch('close', {});
  }

  sendFromBridge(data: string | ArrayBuffer): void {
    this.dispatch('message', { data });
  }

  private dispatch(type: string, event: any): void {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}

function buildFrame(frameType: TciStreamType, sampleRate: number, channels: number, floats: number[]): ArrayBuffer {
  const buffer = new ArrayBuffer(TCI_FRAME_HEADER_BYTES + floats.length * Float32Array.BYTES_PER_ELEMENT);
  const view = new DataView(buffer);
  view.setUint32(4, sampleRate, true);
  view.setUint32(20, floats.length, true);
  view.setUint32(24, frameType, true);
  view.setUint32(28, channels, true);
  new Float32Array(buffer, TCI_FRAME_HEADER_BYTES, floats.length).set(floats);
  return buffer;
}

function buildIqFloats(pairs: number): number[] {
  const floats: number[] = [];
  for (let i = 0; i < pairs; i += 1) {
    floats.push(Math.sin(i / 8), Math.cos(i / 8));
  }
  return floats;
}

describe('/remote-next smoke flow', () => {
  it('handles ready state, IQ, RX audio, TX blocking, and TX release over a fake bridge socket', () => {
    const controller = createRemoteController(env);
    const session = createRemoteSession(controller);
    const socket = new FakeBridgeSocket();
    session.bind(socket);

    socket.sendFromBridge('ready;dds:0,7150000;vfo:0,0,7150000;iq_samplerate:192000;audio_start;trx:0,false;');
    let state = controller.getState();
    expect(state.radio.dds).toBe(7150000);
    expect(state.radio.vfoA).toBe(7150000);
    expect(state.radio.sampleRate).toBe(192000);
    expect(state.radio.audioStreaming).toBe(true);
    expect(state.transport.audioStreaming).toBe(true);

    socket.sendFromBridge(buildFrame(TciStreamType.Iq, 192000, 2, buildIqFloats(128)));
    state = controller.getState();
    expect(state.transport.frameCounter).toBe(1);
    expect(state.transport.iqPackets).toHaveLength(1);

    socket.sendFromBridge(buildFrame(TciStreamType.AudioLeft, 48000, 2, [0.1, 0.1, 0.2, 0.2]));
    state = controller.getState();
    expect(state.transport.audioFramesPlayed).toBe(1);

    socket.sendFromBridge('trx:0,true;');
    socket.sendFromBridge(buildFrame(TciStreamType.Iq, 192000, 2, buildIqFloats(64)));
    socket.sendFromBridge(buildFrame(TciStreamType.AudioLeft, 48000, 2, [0.3, 0.3, 0.4, 0.4]));
    state = controller.getState();
    expect(state.radio.txEnabled).toBe(true);
    expect(state.transport.txEnabled).toBe(true);
    expect(state.transport.frameCounter).toBe(1);
    expect(state.transport.audioFramesPlayed).toBe(1);

    socket.sendFromBridge('trx:0,false;');
    socket.sendFromBridge(buildFrame(TciStreamType.Iq, 192000, 2, buildIqFloats(64)));
    state = controller.getState();
    expect(state.radio.txEnabled).toBe(false);
    expect(state.transport.txEnabled).toBe(false);
    expect(state.transport.frameCounter).toBe(2);
  });
});
