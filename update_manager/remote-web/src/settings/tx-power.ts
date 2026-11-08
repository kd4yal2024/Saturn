/** A slider draft is not a committed host command. Host echoes cannot edit it. */
export class TxPowerDraft {
  private draft: number | null = null;
  preview(value: number, ceiling = 100): number {
    this.draft = Math.max(1, Math.min(ceiling, Math.round(value)));
    return this.draft;
  }
  get editing(): boolean { return this.draft !== null; }
  displayed(committed: number): number { return this.draft ?? committed; }
  commit(value: number, ceiling = 100): number {
    const watts = this.preview(this.draft ?? value, ceiling);
    this.draft = null;
    return watts;
  }
  cancel(): void { this.draft = null; }
}
