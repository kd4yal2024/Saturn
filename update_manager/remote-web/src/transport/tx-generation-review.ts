// Offline reference client, NOT imported by remote-next-entry/installed UI.
// Generation authority is frozen before framing/encoding. Source replacement
// must acknowledge a fresh capture/resampler/partial-packet boundary first.
import { buildTxMicPcmS16Frame } from './tx-uplink';
import { createTxOpusAudioDataFromFloat32, createTxOpusFrameProducer,
  type TxOpusFrameProducer } from '../audio/tx-opus-encoder';

export const TX_GENERATION_REVIEW_DEFAULT_ENABLED = false;
export const PSG1_MAGIC = 0x50534731;
export const PSG1_BANK_PACKETS = 4;
const AGE_MS = 150;
type Codec = 'pcm' | 'opus_wb';
export interface FreshCapture {
  token: string;
  historyResetAtMs: number;
  close(): void;
}
export type CaptureFactory = (token: string,
  deliver: (samples: Float32Array, oldestObservationMs: number) => void) => Promise<FreshCapture>;
interface Pending { sequence: number; oldest: number }
interface Bank { producer: TxOpusFrameProducer; pending: Map<number, Pending>; submitted: number; oldest: number; flushed: boolean }

export function tagPsg1(frame: ArrayBuffer, token: string): ArrayBuffer {
  if (!/^[0-9a-f]{32}$/.test(token) || frame.byteLength < 64) throw new Error('invalid_psg1_tag');
  const copy = frame.slice(0); // caller's already-issued old frame is immutable
  const view = new DataView(copy);
  for (let i = 0; i < 16; i++) view.setUint8(44 + i, parseInt(token.slice(i * 2, i * 2 + 2), 16));
  view.setUint32(60, PSG1_MAGIC, true);
  return copy;
}

export class TxGenerationReviewClient {
  private version = 0;
  private lastOffer = 0n;
  private token: string | null = null;
  private source: FreshCapture | null = null;
  private sequence = 0;
  private nextSend = 0;
  private banks = new Map<number, Bank>();
  private ready = new Map<number, { frame: ArrayBuffer; oldest: number }>();
  constructor(private readonly options: {
    enabled?: boolean;
    codec: Codec;
    scope: unknown;
    now(): number;
    capture: CaptureFactory;
    media(frame: ArrayBuffer): void;
    control(text: string): void;
  }) {}

  // Invalidation is first. Closing capture/encoders is cleanup, not authority.
  private invalidate(): () => void {
    this.version++;
    this.token = null;
    const source = this.source;
    const banks = [...this.banks.values()];
    this.source = null;
    this.banks.clear(); this.ready.clear(); this.sequence = 0; this.nextSend = 0;
    return () => { source?.close(); for (const b of banks) b.producer.close(); };
  }
  release(): void {
    const cleanup = this.invalidate();
    try { this.options.control('trx:0,false;'); } finally { cleanup(); }
  }
  disconnect(): void { this.invalidate()(); }
  private fail(version: number): void { if (version === this.version) this.release(); }

  async generation(text: string): Promise<boolean> {
    const match = /^saturn_tx_generation:psg1,(off|[0-9a-f]{32}),([1-9][0-9]{0,19});$/.exec(text);
    if (!match) return false;
    const serial = BigInt(match[2]!);
    if (serial > 0xffffffffffffffffn || serial < this.lastOffer) return false;
    if (match[1] === this.token) return true;
    if (match[1] !== 'off' && serial === this.lastOffer) return false;
    this.lastOffer = serial;
    this.invalidate()();
    if (match[1] === 'off') return true;
    if (this.options.enabled !== true) return false;
    const token = match[1]!; const version = this.version; const requested = this.options.now();
    this.token = token;
    try {
      const source = await this.options.capture(token, (samples, oldest) => this.input(version, token, samples, oldest));
      if (version !== this.version) { source.close(); return false; }
      if (source.token !== token || !Number.isFinite(source.historyResetAtMs)
        || source.historyResetAtMs < requested || source.historyResetAtMs > this.options.now()) {
        source.close(); this.fail(version); return false;
      }
      this.source = source;
      return true;
    } catch { this.fail(version); return false; }
  }
  private input(version: number, token: string, samples: Float32Array, oldest: number): void {
    if (version !== this.version || token !== this.token || !this.source) return;
    const now = this.options.now();
    const size = this.options.codec === 'pcm' ? 512 : 960;
    if (!Number.isFinite(oldest) || oldest < this.source.historyResetAtMs || oldest > now
      || now - oldest > AGE_MS || samples.length !== size || samples.some(s => !Number.isFinite(s))
      || this.sequence === 0xffffffff) { this.fail(version); return; }
    const seq = this.sequence++;
    if (this.options.codec === 'pcm') {
      this.emit(version, token, seq, buildTxMicPcmS16Frame(samples, seq).frame, oldest);
      return;
    }
    const index = Math.floor(seq / PSG1_BANK_PACKETS);
    let bank = this.banks.get(index);
    if (!bank) {
      if (this.banks.size >= 2) { this.fail(version); return; }
      const status = createTxOpusFrameProducer('opus_wb', { enabled: true, scope: this.options.scope,
        onError: () => { if (this.banks.has(index)) this.fail(version); },
        onFrame: (built, meta) => {
          if (version !== this.version || token !== this.token) return;
          const b = this.banks.get(index);
          if (!b) return; // a closed bank's late/orphan callback is not fresh
          const pending = b.pending.get(meta.sequence);
          if (!pending || meta.chunkDurationUs !== 20_000 || built.decodedSampleCount !== 960) {
            this.fail(version); return;
          }
          b.pending.delete(meta.sequence);
          this.emit(version, token, pending.sequence, built.frame, pending.oldest);
          if (version === this.version && b.flushed && b.pending.size === 0) {
            this.banks.delete(index); b.producer.close();
          }
        },
      });
      if (status.state !== 'ready') { this.fail(version); return; }
      if (version !== this.version || token !== this.token) { status.producer.close(); return; }
      bank = { producer: status.producer, pending: new Map(), submitted: 0, oldest, flushed: false };
      this.banks.set(index, bank);
    }
    bank.oldest = Math.min(bank.oldest, oldest); bank.submitted++;
    bank.pending.set(seq, { sequence: seq, oldest: bank.oldest });
    const data = createTxOpusAudioDataFromFloat32(samples, 'opus_wb', { scope: this.options.scope, sequence: seq });
    if (data.state !== 'ready') { this.fail(version); return; }
    try { bank.producer.encode(data.audioData, seq); } finally { data.audioData.close?.(); }
    if (version === this.version && bank.submitted === PSG1_BANK_PACKETS) {
      const complete = bank;
      void bank.producer.flush().then(() => {
        if (version !== this.version || this.banks.get(index) !== complete) return;
        complete.flushed = true;
        if (complete.pending.size !== 0) { this.fail(version); return; }
        this.banks.delete(index); complete.producer.close();
      }, () => this.fail(version));
    }
  }
  private emit(version: number, token: string, seq: number, frame: ArrayBuffer, oldest: number): void {
    if (version !== this.version || token !== this.token) return;
    if (this.ready.size >= 8 || this.ready.has(seq)) { this.fail(version); return; }
    this.ready.set(seq, { frame: tagPsg1(frame, token), oldest });
    while (this.ready.has(this.nextSend)) {
      const packet = this.ready.get(this.nextSend)!;
      if (this.options.now() - packet.oldest > AGE_MS) { this.fail(version); return; }
      this.ready.delete(this.nextSend++);
      try { this.options.media(packet.frame); } catch { this.fail(version); return; }
    }
  }
}
