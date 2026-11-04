/** Operator-only rear-switch notifications. The Bridge validates each press
 * token independently; a notification never supplies an output generation. */
export class HardwarePttSession {
  private token: string | null = null;
  private last = 0n;
  private owned = false;
  private attempt: object | null = null;
  constructor(private readonly options: {
    canStart: () => boolean;
    transmitting: () => boolean;
    start: (token: string) => Promise<boolean>;
    stop: (token: string) => void;
    changed?: (active: boolean) => void;
    ready?: () => boolean;
  }) {}
  /** Explicit local TX actions supersede this input's pending UI ownership. */
  disown(): void { this.token = null; this.owned = false; this.attempt = null; this.options.changed?.(false); }
  reset(): void {
    const stop = this.owned && this.attempt !== null; const token = this.token;
    this.disown(); this.last = 0n;
    if (stop && token) this.options.stop(token);
  }
  text(text: string): void {
    for (const command of text.split(';').map(s => s.trim())) {
      if (command.startsWith('saturn_ptt_rejected:')) {
        if (command.split(',')[1] !== this.token) continue;
        const stop = this.owned && this.attempt !== null; const token = this.token; this.disown();
        if (stop && token) this.options.stop(token);
        continue;
      }
      const match = /^saturn_ptt:0,(true|false),([1-9][0-9]{0,19})$/.exec(command);
      if (!match || BigInt(match[2]!) > 0xffffffffffffffffn) continue;
      const down = match[1] === 'true'; const token = match[2]!;
      if (!down) {
        if (token !== this.token) continue;
        const stop = this.owned && this.attempt !== null; this.disown();
        if (stop) this.options.stop(token);
        continue;
      }
      const sequence = BigInt(token);
      if (sequence <= this.last) continue;
      this.last = sequence;
      if (!this.options.canStart() || this.options.transmitting()) continue;
      this.token = token; this.owned = true;
      this.options.changed?.(true);
    }
    // Retain the original press token while its already-completed host cleanup
    // report is in transit. No new token, action or source age is manufactured.
    if (!this.owned || !this.token || this.attempt || !this.options.canStart() || this.options.ready?.() === false) return;
    const token = this.token; const attempt = {}; this.attempt = attempt;
    void this.options.start(token).then(started => {
      if (this.attempt === attempt && !started) this.disown();
    }).catch(() => {
      if (this.attempt !== attempt) return;
      this.disown(); this.options.stop(token);
    });
  }
}
