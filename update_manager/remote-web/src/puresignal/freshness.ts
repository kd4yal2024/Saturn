/** Offline M2 consumer. Not yet attached to the production websocket/UI.
 * Clock rates must differ by <= oscillatorBudgetPpm; if that cannot be assumed,
 * return timing-unverified instead of issuing a freshness certificate.
 */
export interface FreshnessStamp {
  bootId: string;
  observedMonoNs: string;
  publishedMonoNs: string;
  sequence: string;
  staleAfterMs: number;
}
export interface ProbeReply {
  nonce: string;
  connectionEpoch: string;
  bootId: string;
  serverReplyMonoNs: string;
}
export type FreshnessResult = {
  validity: "valid" | "stale" | "unavailable";
  reason: string | null;
  maximumAgeMs: number | null;
};
const unavailable = (reason: string): FreshnessResult =>
  ({ validity: "unavailable", reason, maximumAgeMs: null });
const ns = (value: string): bigint => {
  if (value.length > 20 || !/^(0|[1-9][0-9]*)$/.test(value)) throw new Error("invalid unsigned clock");
  const parsed = BigInt(value);
  if (parsed > 18_446_744_073_709_551_615n) throw new Error("unsigned clock overflow");
  return parsed;
};

export class TelemetryFreshness {
  private epoch = "";
  private pending: { nonce: string; sent: bigint } | null = null;
  private bound: { boot: string; upperOffset: bigint; barrier: bigint;
    sent: bigint; received: bigint } | null = null;
  private lastCheck: bigint | null = null;
  private highWater = new Map<string, bigint>();

  constructor(private readonly boundLifetimeNs = 2_000_000_000n,
    private readonly maxProbeRttNs = 200_000_000n,
    private readonly maxConsumerSilenceNs = 500_000_000n,
    private readonly oscillatorBudgetPpm = 1000n) {
    if (boundLifetimeNs <= 0n || maxProbeRttNs <= 0n ||
        maxConsumerSilenceNs <= 0n || oscillatorBudgetPpm < 0n) {
      throw new Error("invalid freshness policy");
    }
  }

  reconnect(epoch: string): void {
    ns(epoch);
    // Connection epochs must be unique, including when the same boot reconnects.
    if (epoch === this.epoch) throw new Error("connection epoch reused");
    this.epoch = epoch;
    this.suspend();
    this.highWater.clear();
  }

  suspend(): void {
    this.bound = null;
    this.pending = null;
    this.lastCheck = null;
  }

  probe(nonce: string, clientSendMonoNs: string): void {
    if (!this.epoch || !nonce || this.pending) throw new Error("probe not ready");
    this.pending = { nonce, sent: ns(clientSendMonoNs) };
  }

  acceptProbe(reply: ProbeReply, clientReceiveMonoNs: string): boolean {
    const pending = this.pending;
    if (!pending || reply.nonce !== pending.nonce ||
        reply.connectionEpoch !== this.epoch || !reply.bootId) return false;
    this.pending = null;
    try {
      const received = ns(clientReceiveMonoNs);
      const server = ns(reply.serverReplyMonoNs);
      const rtt = received - pending.sent;
      if (rtt < 0n || rtt > this.maxProbeRttNs) { this.suspend(); return false; }
      // Server reply must be stamped AFTER receipt of this unpredictable nonce.
      const error = this.drift(rtt);
      if (this.bound?.boot !== reply.bootId) this.highWater.clear();
      this.bound = { boot: reply.bootId, upperOffset: server - pending.sent + error,
        barrier: server, sent: pending.sent, received };
      this.lastCheck = received;
      return true;
    } catch { this.suspend(); return false; }
  }

  check(topic: string, connectionEpoch: string, stamp: FreshnessStamp,
    clientNowMonoNs: string): FreshnessResult {
    try {
      const now = ns(clientNowMonoNs);
      if (connectionEpoch !== this.epoch) return unavailable("connection-mismatch");
      if (this.lastCheck !== null && (now < this.lastCheck ||
          now - this.lastCheck > this.maxConsumerSilenceNs)) this.suspend();
      this.lastCheck = now;
      const bound = this.bound;
      if (!bound || now < bound.received || now - bound.received > this.boundLifetimeNs) {
        this.bound = null;
        return unavailable("timing-unverified");
      }
      if (stamp.bootId !== bound.boot) {
        this.suspend(); return unavailable("generation-mismatch");
      }
      const observed = ns(stamp.observedMonoNs);
      const published = ns(stamp.publishedMonoNs);
      const sequence = ns(stamp.sequence);
      if (!Number.isFinite(stamp.staleAfterMs) || stamp.staleAfterMs <= 0 ||
          published < observed) return unavailable("invalid-stamp");
      if (observed < bound.barrier) return unavailable("before-freshness-barrier");
      if (sequence <= (this.highWater.get(topic) ?? -1n)) return unavailable("reordered");
      const serverNowUpper = now + bound.upperOffset + this.drift(now - bound.sent);
      if (published > serverNowUpper) return unavailable("invalid-stamp");
      const maximumAgeNs = serverNowUpper - observed;
      if (maximumAgeNs < 0n) return unavailable("invalid-stamp");
      this.highWater.set(topic, sequence);
      const maximumAgeMs = Number(maximumAgeNs) / 1e6;
      return maximumAgeMs > stamp.staleAfterMs
        ? { validity: "stale", reason: "stale", maximumAgeMs }
        : { validity: "valid", reason: null, maximumAgeMs };
    } catch { return unavailable("invalid-stamp"); }
  }

  private drift(elapsed: bigint): bigint {
    return (elapsed * this.oscillatorBudgetPpm + 999_999n) / 1_000_000n;
  }
}
