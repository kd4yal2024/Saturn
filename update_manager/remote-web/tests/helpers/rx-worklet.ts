import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const template = readFileSync(new URL('../../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const begin = template.indexOf('const WORKLET_SOURCE = `');
const end = template.indexOf('`;', begin);
if (begin < 0 || end <= begin) throw Error('RX worklet source markers missing or reordered');
const constants: Record<string, number> = { RX_MSG_QUEUE_MAX_PACKETS: 8,
  CTRL_READ_IDX: 0, CTRL_WRITE_IDX: 1, CTRL_GAIN_IDX: 4, CTRL_MUTED_IDX: 5, TX_MIC_BLOCK_SAMPLES: 1024 };
const source = template.slice(begin + 'const WORKLET_SOURCE = `'.length, end)
  .replace(/\$\{([^}]+)\}/g, (_, key: string) => {
    if (!(key in constants)) throw Error('Unknown worklet constant: ' + key);
    return String(constants[key]);
  });

/** Virtual audio time; executes the actual shipped processor with no device. */
export function rxWorkletHarness(boundedPlayback = true, sampleRate = 48000) {
  const messages: any[] = [];
  let Processor: any;
  const context = { sampleRate, currentTime: 0,
    AudioWorkletProcessor: class { port = { postMessage: (m: any) => messages.push(structuredClone(m)) }; },
    registerProcessor: (name: string, ctor: any) => { if (name === 'saturn-rx-player') Processor = ctor; } };
  runInNewContext(source, context);
  if (!Processor) throw Error('RX processor not registered');
  const processor = new Processor({ processorOptions: { boundedPlayback } });
  const tick = () => {
    const out = [new Float32Array(128), new Float32Array(128)];
    processor.process([], [out]); context.currentTime += 128 / sampleRate; return out;
  };
  const samples = (left: Float32Array, right: Float32Array, id = 1) => processor._onMessage({
    type: 'audio', left, right, diagnostic: { id, sentContextMs: context.currentTime * 1000 - 7 } });
  const packet = (frames = 1024, id = 1) => samples(new Float32Array(frames).fill(0.5), new Float32Array(frames).fill(-0.5), id);
  return { processor, messages, tick, packet, samples, context };
}
