import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const source = template.slice(template.indexOf('    const startedSabWorklets ='), template.indexOf('    function scheduleAudioPlayback('));

describe('shipped SAB producer starts each worklet once after data publication', () => {
  it('keeps stereo samples, resumes a flushed ring and starts a replacement node', () => {
    const ctrl = new Uint32Array(new SharedArrayBuffer(32));
    const ring = new Float32Array(new SharedArrayBuffer(4096 * 8));
    const messages: unknown[] = [];
    const node = { port: { postMessage: (message: unknown) => messages.push({ message, write: ctrl[1] }) } };
    const state = { rxRingF32: ring, rxCtrlU32: ctrl, rxWorkletNode: node, rxWorkletDrops: 0 };
    const ctx: any = { state, RX_RING_FRAMES: 4096, CTRL_READ_IDX: 0, CTRL_WRITE_IDX: 1, recordOperatorFault() {}, logEvent() {} };
    runInNewContext(source + '\nthis.write = writeRxAudioFrameToSab;', ctx);
    const left = new Float32Array([.25, .5]), right = new Float32Array([-.25, -.5]);
    expect(ctx.write(left, right)).toBe(true);
    expect(Array.from(ring.slice(0, 4))).toEqual([.25, -.25, .5, -.5]);
    expect(messages).toEqual([{ message: { type: 'stream-start' }, write: 2 }]);
    for (let i = 0; i < 100; i++) { ctrl[0] = ctrl[1]!; ctx.write(left, right); }
    ctrl[0] = ctrl[1] = 0; // same processor flush; started flag remains true
    ctx.write(left, right);
    expect(messages).toHaveLength(1);
    state.rxWorkletNode = { port: { postMessage: node.port.postMessage } };
    ctx.write(left, right);
    expect(messages).toHaveLength(2);
  });
});
