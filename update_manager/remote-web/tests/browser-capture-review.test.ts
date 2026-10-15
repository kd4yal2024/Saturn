import {describe, expect, it} from 'vitest';
import {runInNewContext} from 'node:vm';
import {REVIEW_CAPTURE_WORKLET} from '../src/puresignal/browser-capture-review';

function harness(startFrame: number, endFrame: number, blockSize: number) {
  const messages: Array<{samples?: Float32Array; oldestFrame?: number; tailSamples?: number}> = [];
  let Processor: new (options: unknown) => {process(i: Float32Array[][], o: Float32Array[][]): boolean};
  const scope = {currentFrame: 0, Float32Array,
    AudioWorkletProcessor: class {port = {postMessage(value: unknown) {messages.push(value as typeof messages[number]);}};},
    registerProcessor(_: string, ctor: typeof Processor) {Processor = ctor;}};
  runInNewContext(REVIEW_CAPTURE_WORKLET, scope);
  const processor = new Processor!({processorOptions: {startFrame, endFrame, blockSize}});
  return {messages, process(frame: number, values: number[]) {
    scope.currentFrame = frame;
    const out = new Float32Array(values.length).fill(1);
    const running = processor.process([[new Float32Array(values)]], [[out]]);
    expect(out.every(v => v === 0)).toBe(true);
    return running;
  }};
}
describe('recorded source real-worklet experiment bookkeeping', () => {
  it('excludes pre-start data, retaining oldest frame across partial quanta', () => {
    const h = harness(3, 12, 4);
    h.process(0, [99,99,99,1,2]); expect(h.messages).toHaveLength(0);
    h.process(5, [3,4,5]);
    expect(h.messages[0]!.oldestFrame).toBe(3);
    expect([...h.messages[0]!.samples!]).toEqual([1,2,3,4]);
    h.process(8, [6,7,8,9,99]);
    expect(h.messages[1]!.oldestFrame).toBe(7);
    expect([...h.messages[1]!.samples!]).toEqual([5,6,7,8]);
    expect(h.process(13, [99])).toBe(false);
    expect(h.messages[2]!.tailSamples).toBe(1);
  });
  it('fresh processor has no retained packet/history from a previous generation', () => {
    const old = harness(0, 4, 4); old.process(0, [1,2]);
    const fresh = harness(100, 104, 4); fresh.process(100, [3,4,5,6]);
    expect([...fresh.messages[0]!.samples!]).toEqual([3,4,5,6]);
    expect(fresh.messages[0]!.oldestFrame).toBe(100);
  });
});
