import { HAM_SEGMENTS, visibleBandSegments, type BandSegment } from '../radio/band-plan';
import { HAM_BAND_EDGES_HZ, SWL_BANDS } from '../radio/band';
import { SWL_SOURCE, readSwlResponse, decodeSwlCsv, parseSwlCsv, swlMarkersInView, layoutSwlLabels,
  type SwlCatalogue } from '../radio/swl-schedule';

export type BandGuideView = {
  low: number; high: number; width: number;
  segments: boolean; stations: boolean; scheduledOnly: boolean;
};
const broadcast: readonly BandSegment[] = SWL_BANDS.map(b => ({
  start: b.start, end: b.end, band: `SWL ${b.label}`, label: 'Broadcast', use: 'broadcast',
}));
const navigation: readonly BandSegment[] = HAM_BAND_EDGES_HZ.filter(b => ['11m', 'FM'].includes(b.label))
  .map((b): BandSegment => ({ ...b, band: b.label, label: b.label === 'FM' ? 'Broadcast' : 'Operating memory',
    use: b.label === 'FM' ? 'broadcast' : 'mixed' }));

/** A separate, bounded annotation surface. No IQ, DSP or radio commands. */
export class PanadapterBandGuide {
  private readonly spectrum: HTMLElement;
  private readonly waterfall: HTMLElement;
  private readonly stationLayer: HTMLElement;
  private catalogue: SwlCatalogue | null = null;
  private loading = false;
  private error = '';
  private revision = 0;
  private signature = '';
  private pending = 0;
  private view: BandGuideView | null = null;
  private clock: ReturnType<typeof setTimeout> | null = null;
  private disposed = false;
  private request: AbortController | null = null;

  constructor(private readonly spectrumShell: HTMLElement, private readonly waterfallShell: HTMLElement,
    private readonly status: HTMLElement) {
    this.spectrum = this.layer(spectrumShell, 'band-guide-layer');
    this.waterfall = this.layer(waterfallShell, 'band-guide-layer band-guide-waterfall');
    this.stationLayer = this.layer(this.spectrum, 'swl-station-layer');
    this.scheduleClock();
    document.addEventListener('visibilitychange', this.onVisibility);
    window.addEventListener('pagehide', event => { if (!event.persisted) this.dispose(); });
  }
  private layer(parent: HTMLElement, className: string): HTMLElement {
    const layer = document.createElement('div'); layer.className = className;
    parent.appendChild(layer); return layer;
  }
  private onVisibility = () => { if (!document.hidden) this.queue(); };
  private scheduleClock(): void {
    this.clock = setTimeout(() => {
      if (!document.hidden) this.queue();
      if (!this.disposed) this.scheduleClock();
    }, 60_000 - Date.now() % 60_000 + 20);
  }
  dispose = (): void => {
    this.disposed = true;
    if (this.clock !== null) clearTimeout(this.clock);
    if (this.pending) cancelAnimationFrame(this.pending);
    this.request?.abort();
    document.removeEventListener('visibilitychange', this.onVisibility);
  };
  update(view: BandGuideView): void {
    const previous = this.view;
    if (previous && previous.low === view.low && previous.high === view.high && previous.width === view.width
      && previous.segments === view.segments && previous.stations === view.stations
      && previous.scheduledOnly === view.scheduledOnly) return;
    if (previous && !previous.stations && view.stations) this.error = '';
    this.view = view;
    this.queue();
    if (view.stations && !this.loading && !this.catalogue && !this.error) void this.load();
  }
  private queue(): void {
    if (this.disposed || this.pending) return;
    this.pending = requestAnimationFrame(() => { this.pending = 0; this.draw(); });
  }
  private async load(): Promise<void> {
    this.loading = true; this.queue();
    this.request = new AbortController();
    const timeout = setTimeout(() => this.request?.abort(), 15_000);
    try {
      const response = await fetch('/remote-assets/swl.csv', { credentials: 'same-origin', signal: this.request.signal });
      if (!response.ok) throw Error(`CSV unavailable (${response.status})`);
      const bytes = await readSwlResponse(response);
      this.catalogue = await parseSwlCsv(decodeSwlCsv(bytes));
    } catch (error) {
      this.error = error instanceof Error ? error.message : 'CSV unavailable';
    } finally {
      clearTimeout(timeout); this.request = null;
      this.loading = false; this.revision++; this.queue();
    }
  }
  private draw(): void {
    const view = this.view;
    if (!view || !Number.isFinite(view.low) || !Number.isFinite(view.high) || view.high <= view.low) return;
    const now = new Date();
    const key = [view.low, view.high, Math.round(view.width), view.segments, view.stations,
      view.scheduledOnly, Math.floor(now.getTime() / 60_000), this.revision, this.loading].join('|');
    if (key === this.signature) return;
    this.signature = key;
    for (const shell of [this.spectrumShell, this.waterfallShell]) shell.classList.toggle('scope-has-band-guide', view.segments);
    for (const layer of [this.spectrum, this.waterfall]) {
      for (const rail of layer.querySelectorAll('.band-guide-rail')) rail.remove();
      if (!view.segments) continue;
      this.rail(layer, HAM_SEGMENTS, 0, view, 'US ham operating guide; not license-class privileges');
      this.rail(layer, [...broadcast, ...navigation], 1, view, 'Broadcast / operating-memory ranges');
    }
    this.stationLayer.replaceChildren();
    const seasonExpired = now.toISOString().slice(0, 10) < SWL_SOURCE.validFrom
      || now.toISOString().slice(0, 10) >= SWL_SOURCE.validUntil;
    let stationSummary = '';
    if (view.stations && this.catalogue) {
      const markers = swlMarkersInView(this.catalogue, view.low, view.high, now, view.scheduledOnly);
      const labels = layoutSwlLabels(markers, view.low, view.high, view.width);
      const nodes = document.createDocumentFragment();
      for (const marker of labels) {
        const tick = document.createElement('span'); tick.className = 'swl-station-tick';
        tick.style.left = `${(marker.hz - view.low) / (view.high - view.low) * 100}%`;
        const details = marker.entries.slice(0, 12).map(e => `${e.name} · ${e.country} · ${e.language || '—'} · ${e.time} UTC · ${e.days || 'daily'}`).join('\n');
        tick.title = `${(marker.hz / 1000).toFixed(3)} kHz\n${details}\n${SWL_SOURCE.name} · ${SWL_SOURCE.updated}${seasonExpired ? ' · historical season' : ''}\nScheduled listing, not a signal detection`;
        nodes.appendChild(tick);
        if (marker.lane < 0) continue;
        const label = document.createElement('span'); label.className = 'swl-station-label';
        label.textContent = marker.label; label.title = tick.title;
        label.style.left = `${marker.left / view.width * 100}%`;
        label.style.width = `${marker.width}px`; label.style.top = `${marker.lane * 19}px`;
        nodes.appendChild(label);
      }
      this.stationLayer.appendChild(nodes);
      stationSummary = `${markers.length} ${view.scheduledOnly ? 'scheduled' : 'listed'} frequencies in view`;
    }
    this.stationLayer.hidden = !view.stations;
    this.spectrum.classList.toggle('band-guide-no-segments', !view.segments);
    this.spectrum.hidden = !view.segments && !view.stations;
    this.waterfall.hidden = !view.segments;
    this.status.textContent = `${SWL_SOURCE.name} · ${SWL_SOURCE.updated} · ${this.loading ? 'Loading…'
      : this.error ? this.error : this.catalogue ? `${this.catalogue.entries.length.toLocaleString()} entries${stationSummary ? ` · ${stationSummary}` : ''}` : 'Station labels off'}${seasonExpired ? ' · historical season' : ''}`;
    this.status.title = `${SWL_SOURCE.attribution}\n${SWL_SOURCE.url}\nUTC schedule only; reception is not verified. US ham colors are operating guidance, not license-class privileges.`;
  }
  private rail(parent: HTMLElement, segments: readonly BandSegment[], lane: number,
    view: BandGuideView, description: string): void {
    const rail = this.layer(parent, 'band-guide-rail'); rail.style.top = `${lane * 17}px`;
    rail.setAttribute('aria-label', description);
    for (const segment of visibleBandSegments(segments, view.low, view.high)) {
      const node = document.createElement('span'); node.className = `band-guide-segment band-use-${segment.use}`;
      node.style.left = `${segment.left}%`; node.style.width = `${segment.width}%`;
      node.title = `${segment.band} · ${segment.label}\n${(segment.start / 1e6).toFixed(6)}–${(segment.end / 1e6).toFixed(6)} MHz\n${description}`;
      node.setAttribute('aria-label', node.title);
      if (segment.width / 100 * view.width >= 64) node.textContent = `${segment.band} · ${segment.label}`;
      rail.appendChild(node);
    }
  }
}
