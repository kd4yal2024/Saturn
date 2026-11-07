/** Thetis PA-profile attenuation convention; larger dB means less drive. */
export const PA_BANDS = ['160m','80m','60m','40m','30m','20m','17m','15m','12m','11m','10m','6m'] as const;
export const G2_PA_STARTING_GAINS = [47.9,50.5,50.8,50.8,50.9,50.9,50.5,47,47.9,46.5,46.5,44.6] as const;
export function normalizePaCalibration(value: unknown): number[] | null {
  return Array.isArray(value) && value.length === PA_BANDS.length
    && value.every(v => typeof v === 'number' && Number.isFinite(v) && v >= 40 && v <= 80)
    ? [...value] : null;
}
export function paCalibrationCommand(value: number[] | null): string {
  const gains = normalizePaCalibration(value);
  return `tx_pa_calibration:0,${gains ? gains.join('|') : 'legacy'};`;
}
export function parsePaCalibration(value: string): number[] | null | undefined {
  if (value === 'legacy') return null;
  const fields = value.split('|');
  if (fields.some(v => !v.trim())) return undefined;
  return normalizePaCalibration(fields.map(Number)) ?? undefined;
}

type View = {
  requested: number[] | null;
  host: number[] | null | undefined;
  drive: string | undefined;
  error: string | undefined;
  canApply: boolean;
};

/** Draft edits do not reach the radio until Apply. Display is never an RF meter. */
export class PaCalibrationPanel {
  private draft: number[] | null = null;
  private dirty = false;
  private view: View | null = null;
  constructor(private root: HTMLElement, private apply: (profile: number[] | null) => void) {
    const band = this.field<HTMLSelectElement>('band');
    for (const name of PA_BANDS) { const option = document.createElement('option'); option.text = name; band.add(option); }
    band.addEventListener('change', () => this.paint());
    this.field('defaults').addEventListener('click', () => { this.draft = [...G2_PA_STARTING_GAINS]; this.dirty = true; this.paint(); });
    this.field('legacy').addEventListener('click', () => { this.draft = null; this.dirty = true; this.paint(); });
    this.field<HTMLInputElement>('gain').addEventListener('change', () => {
      const input = this.field<HTMLInputElement>('gain');
      const n = Number(input.value);
      if (this.draft && input.value.trim() && Number.isFinite(n) && n >= 40 && n <= 80) {
        this.draft[band.selectedIndex] = Math.round(n * 100) / 100; this.dirty = true;
      }
      input.value = this.draft ? String(this.draft[band.selectedIndex]) : '';
      this.paint();
    });
    this.field('apply').addEventListener('click', () => {
      if (!this.view?.canApply) return;
      this.apply(this.draft ? [...this.draft] : null); this.dirty = false;
    });
  }
  private field<T extends HTMLElement = HTMLElement>(name: string): T {
    return this.root.querySelector<T>(`[data-pa="${name}"]`)!;
  }
  update(view: View): void {
    this.view = view;
    if (!this.dirty) this.draft = normalizePaCalibration(view.requested);
    this.paint();
  }
  private paint(): void {
    const gain = this.field<HTMLInputElement>('gain');
    gain.disabled = !this.draft;
    if (document.activeElement !== gain) gain.value = this.draft ? String(this.draft[this.field<HTMLSelectElement>('band').selectedIndex]) : '';
    this.field<HTMLButtonElement>('apply').disabled = !this.view?.canApply;
    const requested = this.draft ? 'Band PA gains' : 'Legacy curve';
    const host = this.view?.host === undefined ? 'host not confirmed' : this.view.host ? 'host: band gains' : 'host: legacy curve';
    const pending = this.view?.host !== undefined && JSON.stringify(this.view.requested) !== JSON.stringify(this.view.host);
    this.field('status').textContent = `${requested}${this.dirty ? ' · draft' : ''} · ${host}${pending ? ' · requested profile differs' : ''}${this.view?.drive ? ` · ${this.view.drive}` : ''}${this.view?.error ? ` · ${this.view.error}` : ''}`;
  }
}
