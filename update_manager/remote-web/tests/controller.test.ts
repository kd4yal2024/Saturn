import { describe, expect, it } from 'vitest';
import { createRemoteController } from '../src/controller/create-controller';
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

describe('remote controller', () => {
  it('initializes from remote settings', () => {
    const controller = createRemoteController(env);
    const state = controller.applyRemoteSettings({
      displayZoom: 4,
      radioPrefs: {
        mode: 'DIGU',
        txDrive: 75,
      },
    });
    expect(state.settings.displayZoom).toBe(4);
    expect(state.radio.mode).toBe('DIGU');
    expect(state.radio.txDrive).toBe(75);
    expect(state.transport.displayZoom).toBe(4);
  });

  it('applies TCI text into radio and transport state', () => {
    const controller = createRemoteController(env);
    const result = controller.applyTciText('ready;trx:0,true;audio_start;dds:0,7150000;');
    expect(result.events.ready).toBe(true);
    expect(result.events.displayCenterHzChanged).toBe(true);
    expect(result.state.radio.dds).toBe(7150000);
    expect(result.state.radio.txEnabled).toBe(true);
    expect(result.state.transport.txEnabled).toBe(true);
    expect(result.state.transport.audioStreaming).toBe(true);
  });

  it('applies iq binary frames through the controller', () => {
    const controller = createRemoteController(env);
    const buffer = buildFrame(0, 192000, 2, [1, 2, 3, 4]);
    const result = controller.applyBinaryFrame(buffer, 3000);
    expect(result.kind).toBe('iq');
    expect(result.accepted).toBe(true);
    expect(result.state.transport.iqPackets).toHaveLength(1);
    expect(result.receivedIqLogMessage).toContain('Received IQ frame');
  });

  it('applies audio frames only when audio streaming is enabled', () => {
    const controller = createRemoteController(env);
    const buffer = buildFrame(1, 48000, 2, [0.1, 0, 0.2, 0]);
    let result = controller.applyBinaryFrame(buffer, 100);
    expect(result.kind).toBe('audio');
    expect(result.accepted).toBe(false);

    controller.applyTciText('audio_start;');
    result = controller.applyBinaryFrame(buffer, 200);
    expect(result.accepted).toBe(true);
    expect(result.state.transport.audioFramesPlayed).toBe(1);
  });
});
