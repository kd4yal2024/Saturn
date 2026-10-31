/** Imported offline evidence only. No TCI, audio, TX controls or authority. */
export interface SignalSummary {
  format: 'saturn-signal-diagnostics-v1'; session: string; generation: string; config: string; txEpoch: string;
  point: string; backend: string; event: string; timeDomain: string;
  observedFirstNs: string | null; observedLastNs: string | null; recordedNs: string;
  rateHz: number; analysisAvailable: boolean; analysisReason?: string;
  componentPeak?: number; rms?: number; dcMagnitude?: number; silent?: boolean;
  componentsAtFullScale?: number; toneHz?: number[]; toneAmplitude?: number[]; toneEstimatesResolved?: boolean;
  continuity: string; overload: string; keyedSnapshot: boolean | null;
  requestedBytes: string | null; completedBytes: string | null;
  rfOutputVerified: false; rfImdQualified: false; continuousKeyedStateVerified: false;
  deliveryAgeVerified: false;
}
const points = ['PreWdspPcm', 'WdspIq', 'BeforeOutputIq', 'ReturnedReferenceIq', 'ReturnedRfIq', 'Timeline'];
const events = ['Samples', 'OperatorRequest', 'SourceSelected', 'FifoPrepared', 'KeyAsserted',
  'ControlChanged', 'OutputCompleted', 'StopRequested', 'CleanupCompleted', 'Rejected'];
const integerText = (v: unknown) => typeof v === 'string' && /^\d{1,20}$/.test(v)
  && BigInt(v) <= 18446744073709551615n;
const idText = (v: unknown) => typeof v === 'string' && /^\d{1,39}$/.test(v)
  && BigInt(v) <= 340282366920938463463374607431768211455n;
const finite = (v: unknown) => typeof v === 'number' && Number.isFinite(v);
export function parseSignalDiagnostics(text: string): SignalSummary[] {
  if (text.length > 4_000_000) throw new Error('Diagnostic report exceeds 4 MB limit');
  const reports: unknown = JSON.parse(text);
  if (!Array.isArray(reports) || !reports.length || reports.length > 256)
    throw new Error('Expected 1–256 diagnostic records; empty evidence is unavailable');
  return reports.map((value: unknown) => {
    const r = value as SignalSummary;
    if (!r || r.format !== 'saturn-signal-diagnostics-v1' || !points.includes(r.point)
      || !events.includes(r.event) || !['P2', 'DirectXdma'].includes(r.backend)
      || !['BridgeIngress', 'HostProcessing', 'OfflineFixture'].includes(r.timeDomain)
      || ![r.session, r.config].every(idText) || ![r.generation, r.recordedNs, r.txEpoch].every(integerText)
      || ![r.observedFirstNs, r.observedLastNs, r.requestedBytes, r.completedBytes].every(v => v === null || integerText(v))
      || !Number.isSafeInteger(r.rateHz) || r.rateHz < 0 || r.rateHz > 10_000_000
      || !['Unknown', 'Clear', 'Fault'].includes(r.continuity) || !['Unknown', 'Clear', 'Fault'].includes(r.overload)
      || ![true, false, null].includes(r.keyedSnapshot)
      || typeof r.analysisAvailable !== 'boolean'
      || r.rfOutputVerified !== false || r.rfImdQualified !== false
      || r.continuousKeyedStateVerified !== false || r.deliveryAgeVerified !== false)
      throw new Error('Unsupported or invalid diagnostic record');
    if (r.observedFirstNs !== null && r.observedLastNs !== null
      && (BigInt(r.observedFirstNs) > BigInt(r.observedLastNs) || BigInt(r.observedLastNs) > BigInt(r.recordedNs)))
      throw new Error('Invalid original observation interval');
    if (r.analysisAvailable && (!finite(r.componentPeak) || r.componentPeak! < 0 || !finite(r.rms) || r.rms! < 0
      || !finite(r.dcMagnitude) || r.dcMagnitude! < 0 || typeof r.silent !== 'boolean'
      || typeof r.toneEstimatesResolved !== 'boolean'
      || !Number.isSafeInteger(r.componentsAtFullScale) || r.componentsAtFullScale! < 0
      || !Array.isArray(r.toneHz) || r.toneHz.length !== 2 || !r.toneHz.every(finite)
      || !Array.isArray(r.toneAmplitude) || r.toneAmplitude.length !== 2
      || !r.toneAmplitude.every(v => finite(v) && v >= 0)))
      throw new Error('Invalid diagnostic levels');
    if (!r.analysisAvailable && (typeof r.analysisReason !== 'string' || r.analysisReason.length > 200))
      throw new Error('Missing unavailable reason');
    return r;
  });
}
const dbfs = (v: number) => v === 0 ? '−∞ dBFS' : `${(20 * Math.log10(v)).toFixed(1)} dBFS`;
export function signalSummaryText(records: SignalSummary[]): string {
  const heading = 'Imported capture — NOT live telemetry. Observation/delivery age unverified.\n'
    + 'Digital levels only; physical RF, continuous keying and calibrated RF IMD remain unqualified.\n';
  return heading + records.map(r => {
    const context = `${r.backend} / ${r.point} / ${r.event}\nSession ${r.session}, generation ${r.generation}, config ${r.config}, TX epoch ${r.txEpoch}; ${r.timeDomain}\n`
      + `Original interval ${r.observedFirstNs ?? 'unknown'}…${r.observedLastNs ?? 'unknown'} ns; recorded ${r.recordedNs} ns\n`
      + `Continuity ${r.continuity}; overload ${r.overload}; keyed snapshot ${r.keyedSnapshot ?? 'unknown'}\n`;
    const levels = r.analysisAvailable
      ? `Component peak ${dbfs(r.componentPeak!)}; ${r.point === 'PreWdspPcm' ? 'PCM' : 'Complex'} RMS ${dbfs(r.rms!)}; DC ${dbfs(r.dcMagnitude!)}; at/full-scale components ${r.componentsAtFullScale}\n`
        + (r.toneEstimatesResolved ? `${r.toneHz![0]} Hz: ${dbfs(r.toneAmplitude![0]!)}; ${r.toneHz![1]} Hz: ${dbfs(r.toneAmplitude![1]!)}${r.silent ? '; SILENCE' : ''}`
          : 'Tone estimates unresolved at this window length; retain a longer contiguous capture.')
      : `Analysis unavailable: ${r.analysisReason}`;
    const completion = r.event === 'OutputCompleted'
      ? `\nCompleted ${r.completedBytes ?? 'unknown'} / ${r.requestedBytes ?? 'unknown'} bytes (${r.backend === 'P2' ? 'socket completion, not DMA' : 'H2C or file-fixture completion; see time domain'})` : '';
    return context + levels + completion;
  }).join('\n\n');
}
export function mountSignalMonitor(root: HTMLElement): void {
  const input = root.querySelector<HTMLInputElement>('[data-ps-signal-file]');
  const output = root.querySelector<HTMLElement>('[data-ps-signal-summary]');
  const pop = root.querySelector<HTMLButtonElement>('[data-ps-signal-popout]');
  if (!input || !output || !pop || root.dataset.mounted === 'true') return;
  root.dataset.mounted = 'true';
  let imported = 'No diagnostic capture loaded. Live levels unavailable.';
  let selection = 0;
  input.addEventListener('change', () => {
    const selected = ++selection;
    const file = input.files?.[0];
    if (!file) return;
    if (file.size > 4_000_000) {
      imported = 'Diagnostic report exceeds 4 MB limit'; output.textContent = imported; return;
    }
    void file.text().then(text => {
      if (selected !== selection) return;
      imported = signalSummaryText(parseSignalDiagnostics(text)); output.textContent = imported;
    }).catch((error: unknown) => {
      if (selected !== selection) return;
      imported = `Diagnostic capture unavailable: ${error instanceof Error ? error.message : 'invalid file'}`;
      output.textContent = imported;
    });
  });
  pop.addEventListener('click', () => {
    const child = window.open('', '_blank', 'popup,width=750,height=650');
    if (!child) { output.textContent = 'Pop-out blocked by browser; imported capture remains in this panel.'; return; }
    child.opener = null; child.document.title = 'Saturn PS Monitor — imported evidence';
    const pre = child.document.createElement('pre'); pre.style.whiteSpace = 'pre-wrap';
    pre.textContent = imported; child.document.body.replaceChildren(pre);
  });
}
