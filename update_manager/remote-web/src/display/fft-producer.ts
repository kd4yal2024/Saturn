import InlineFftWorker from './fft-worker?worker&inline';
import type { CompletedSpectrum, DisplayRequest, FftJob, FftReply } from './fft-protocol';

export interface WorkerPort {
  onmessage: ((event: MessageEvent<FftReply>) => void) | null;
  onerror: ((event: ErrorEvent) => unknown) | null;
  postMessage(job: FftJob, transfer: ArrayBuffer[]): void;
  terminate(): void;
}
export interface ProducerOptions {
  createWorker?: () => WorkerPort;
  now?: () => number;
  schedule?: (callback: () => void, delay: number) => number;
  cancel?: (id: number) => void;
  enabled?: boolean;
  timeoutMs?: number;
  onFailure?: (reason: string) => void;
}
interface Slot { input: ArrayBuffer | null; output: ArrayBuffer | null; state: 'free' | 'worker' | 'ready' }
interface Flight { slot: Slot; request: DisplayRequest; id: number; generation: number; postedAt: number }
const MAX_FFT = 16384, SLOT_COUNT = 2, SAMPLE_CAP = 240;
const sample = (values: number[], value: number) => { values.push(value); if (values.length > SAMPLE_CAP) values.shift(); };
const stats = (values: number[]) => {
  const a = values.slice().sort((a,b) => a-b);
  const at = (p: number) => a.length ? a[Math.min(a.length-1, Math.ceil(a.length*p)-1)]! : null;
  return { count:a.length, p50:at(.5), p95:at(.95), p99:at(.99), max:a.at(-1) ?? null };
};

/** One worker job + one replaceable request + one latest completed spectrum.
 * Two fixed slots circulate main -> worker -> main. The pending request holds
 * only metadata and a source reader, never a newly allocated sample window.
 * Transfer ownership is explicit; incoming radio/capture buffers never transfer.
 */
export class DisplayFftProducer {
  private readonly clock: () => number;
  private readonly schedule: (callback: () => void, delay: number) => number;
  private readonly cancel: (id: number) => void;
  private worker: WorkerPort | null = null;
  private slots: Slot[] = [];
  private flight: Flight | null = null;
  private pending: DisplayRequest | null = null;
  private completed: (CompletedSpectrum & { slot: Slot }) | null = null;
  private dueTimer: number | null = null;
  private timeout: number | null = null;
  private nextDueAt = -Infinity;
  private generation = 0;
  private key = '';
  private nextId = 0;
  private lastDrawnId = 0;
  private disposed = false;
  private failed = '';
  private counters = { offered:0, submitted:0, completed:0, coalesced:0, obsolete:0, drawn:0, queueHighWater:0,
    backingAllocations:0, backingAllocationBytes:0, copiedInputBytes:0, abandonedInFlightBuffers:0, invalidatedPending:0, replacedUndrawn:0 };
  private workerWorkspace = {allocations:0, bytes:0};
  private durations = { fft:[] as number[], copy:[] as number[], roundTrip:[] as number[], queueAge:[] as number[], newestToDraw:[] as number[], oldestToDraw:[] as number[] };

  constructor(private options: ProducerOptions = {}) {
    this.clock = options.now || (() => performance.now());
    this.schedule = options.schedule || ((callback,delay) => window.setTimeout(callback,delay));
    this.cancel = options.cancel || (id => window.clearTimeout(id));
    if (options.enabled === false) this.failed = 'Main-thread display selected';
  }
  get available(): boolean { return !this.disposed && !this.failed; }
  get fallbackReason(): string { return this.failed; }

  /** An immediate context boundary, even if no newer IQ packet arrives. */
  invalidate(key = ''): void {
    if (this.pending) this.counters.invalidatedPending++;
    this.generation++; this.key = key; this.pending = null;
    if (this.completed) this.completed.slot.state = 'free';
    this.completed = null;
    if (this.dueTimer !== null) this.cancel(this.dueTimer);
    this.dueTimer = null; this.nextDueAt = -Infinity;
    // Already-transferred buffers stay worker-owned until its matching reply.
  }
  context(key: string): void { if (key !== this.key) this.invalidate(key); }

  request(request: DisplayRequest): boolean {
    if (!this.available) return false;
    if (!Number.isInteger(request.fftSize) || request.fftSize < 32 || request.fftSize > MAX_FFT
      || (request.fftSize & (request.fftSize - 1)) !== 0 || !Number.isInteger(request.floatCount)
      || request.floatCount % 2 !== 0 || request.floatCount < 2*request.fftSize
      || request.floatCount > 2*MAX_FFT || !Number.isFinite(request.newestIngressMs)
      || !Number.isFinite(request.oldestIngressMs) || request.oldestIngressMs > request.newestIngressMs
      || !Number.isFinite(request.intervalMs) || request.intervalMs < 0) {
      this.fail('Unsupported display FFT request; retaining the main-thread path'); return false;
    }
    this.context(request.key);
    this.counters.offered++;
    if (this.pending) this.counters.coalesced++;
    this.pending = {...request};
    this.counters.queueHighWater = Math.max(this.counters.queueHighWater, 1);
    this.pump();
    return this.available;
  }

  /** Borrow only for a synchronous render/capture; never retain these bins. */
  latest(): CompletedSpectrum | null { return this.completed; }
  drawn(result: CompletedSpectrum, now = this.clock()): void {
    if (this.completed !== result || result.generation !== this.generation || result.id === this.lastDrawnId) return;
    this.lastDrawnId = result.id;
    this.counters.drawn++;
    sample(this.durations.newestToDraw, Math.max(0,now-result.newestIngressMs));
    sample(this.durations.oldestToDraw, Math.max(0,now-result.oldestIngressMs));
  }

  private startWorker(): boolean {
    if (this.worker) return true;
    try {
      this.worker = this.options.createWorker ? this.options.createWorker() : new InlineFftWorker({name:'saturn-display-fft'});
      this.worker.onmessage = event => this.receive(event.data);
      this.worker.onerror = () => this.fail('Display FFT worker failed');
      this.slots = Array.from({length:SLOT_COUNT}, () => ({
        input:new ArrayBuffer(MAX_FFT*2*4), output:new ArrayBuffer(MAX_FFT*4), state:'free' as const,
      }));
      this.counters.backingAllocations += SLOT_COUNT*2;
      this.counters.backingAllocationBytes += SLOT_COUNT*MAX_FFT*3*4;
      return true;
    } catch (error) { this.fail('Display FFT worker unavailable: '+String(error)); return false; }
  }

  private pump(): void {
    if (!this.available || this.flight || !this.pending || !this.startWorker()) return;
    const wait = this.nextDueAt-this.clock();
    if (wait > 0) {
      if (this.dueTimer === null) this.dueTimer = this.schedule(() => { this.dueTimer=null;this.pump(); },wait);
      return;
    }
    const slot = this.slots.find(slot => slot.state === 'free');
    if (!slot || !slot.input || !slot.output) { this.fail('Display transfer-pool ownership mismatch'); return; }
    const request = this.pending; this.pending = null;
    const copyAt = this.clock();
    try {
      const written = request.write(new Float32Array(slot.input));
      if (written !== request.floatCount) throw Error('Source window changed before submission');
    } catch (error) { this.fail('Display input copy failed: '+String(error)); return; }
    sample(this.durations.copy, this.clock()-copyAt);
    this.counters.copiedInputBytes += request.floatCount*4;
    const postedAt = this.clock(), id = ++this.nextId;
    this.flight = {slot,request,id,generation:this.generation,postedAt};
    // Anchor cadence to source ingress, not a late dispatch. A busy main
    // thread must not permanently shift production behind live input.
    this.nextDueAt = request.newestIngressMs+request.intervalMs;
    this.counters.submitted++;
    sample(this.durations.queueAge, Math.max(0,postedAt-request.newestIngressMs));
    const job: FftJob = {kind:'fft',id,generation:this.generation,fftSize:request.fftSize,floatCount:request.floatCount,input:slot.input,output:slot.output};
    slot.state = 'worker'; slot.input = null; slot.output = null;
    this.timeout = this.schedule(() => this.fail('Display FFT worker deadline exceeded'), this.options.timeoutMs ?? 2000);
    try { this.worker!.postMessage(job,[job.input,job.output]); }
    catch (error) { this.fail('Display FFT transfer failed: '+String(error)); }
  }

  private receive(reply: FftReply): void {
    if (!this.available) return;
    const flight = this.flight;
    if (!flight || reply.kind !== 'result' || reply.id !== flight.id || reply.generation !== flight.generation
      || !(reply.input instanceof ArrayBuffer) || !(reply.output instanceof ArrayBuffer)
      || reply.input.byteLength !== MAX_FFT*2*4 || reply.output.byteLength !== MAX_FFT*4
      || reply.fftSize !== flight.request.fftSize || reply.floatCount !== flight.request.floatCount
      || !Number.isFinite(reply.fftMs) || reply.fftMs < 0) { this.fail('Invalid display FFT reply'); return; }
    if (this.timeout !== null) this.cancel(this.timeout);
    this.timeout = null; this.flight = null;
    flight.slot.input = reply.input; flight.slot.output = reply.output; flight.slot.state = 'free';
    if (reply.error) { this.fail('Display FFT worker rejected input: '+reply.error); return; }
    const now = this.clock();
    sample(this.durations.fft, reply.fftMs); sample(this.durations.roundTrip, now-flight.postedAt);
    this.counters.completed++;
    this.workerWorkspace.allocations = reply.workspaceAllocations ?? this.workerWorkspace.allocations;
    this.workerWorkspace.bytes = reply.workspaceBytes ?? this.workerWorkspace.bytes;
    if (flight.generation === this.generation) {
      if (this.completed) {
        if (this.completed.id !== this.lastDrawnId) this.counters.replacedUndrawn++;
        this.completed.slot.state = 'free';
      }
      flight.slot.state = 'ready';
      this.completed = {id:flight.id,generation:flight.generation,sequence:flight.request.sequence,fftSize:reply.fftSize,
        newestIngressMs:flight.request.newestIngressMs,oldestIngressMs:flight.request.oldestIngressMs,
        completedAtMs:now,bins:new Float32Array(reply.output,0,reply.fftSize),slot:flight.slot};
    } else this.counters.obsolete++;
    // No FIFO backlog: immediately select the newest pending request. A valid
    // in-flight completion may still be displayed; dropping every completion
    // just because newer IQ exists would starve slow devices indefinitely.
    this.pump();
  }

  private fail(reason: string): void {
    if (this.failed || this.disposed) return;
    this.failed = reason;
    if (this.flight) this.counters.abandonedInFlightBuffers += 2;
    this.invalidate(); this.flight = null;
    if (this.timeout !== null) this.cancel(this.timeout);
    this.timeout = null;
    if (this.worker) { this.worker.onmessage=null;this.worker.onerror=null;this.worker.terminate();this.worker=null; }
    this.slots = [];
    this.options.onFailure?.(reason);
  }
  dispose(): void {
    if (this.disposed) return;
    this.invalidate(); this.disposed = true;
    if (this.timeout !== null) this.cancel(this.timeout);
    this.timeout = null;
    if (this.worker) { this.worker.onmessage=null;this.worker.onerror=null;this.worker.terminate();this.worker=null; }
    this.flight=null;this.slots=[];
  }
  diagnostics(): Record<string, unknown> {
    return {engine:this.available?'worker':'main',fallbackReason:this.failed,generation:this.generation,
      inFlight:this.flight?1:0,pending:this.pending?1:0,ready:this.completed?1:0,
      freeSlots:this.slots.filter(slot=>slot.state==='free').length,slotCount:this.slots.length,
      latestCompletedIngressAgeMs:this.completed?Math.max(0,this.clock()-this.completed.newestIngressMs):null,
      retainedTransferCapacityBytes:this.slots.length*MAX_FFT*3*4,
      timingScope:'browser ingress; worker FFT duration and main round trip use separate local durations; no RF age claim',
      workerWorkspaceAllocations:this.workerWorkspace.allocations, workerWorkspaceAllocationBytes:this.workerWorkspace.bytes,
      ...this.counters,fftMs:stats(this.durations.fft),inputCopyMs:stats(this.durations.copy),roundTripMs:stats(this.durations.roundTrip),
      ingressToPostMs:stats(this.durations.queueAge),newestIngressToDrawMs:stats(this.durations.newestToDraw),oldestIngressToDrawMs:stats(this.durations.oldestToDraw)};
  }
}
