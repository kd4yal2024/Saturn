export const SWL_SOURCE = Object.freeze({
  name: 'EiBi B26', updated: '2026-10-31',
  attribution: 'Eike Bierwirth / EiBi',
  url: 'http://www.eibispace.de/dx/sked-b26.csv',
  validFrom: '2026-10-25', validUntil: '2027-03-28',
});
export const MAX_SWL_BYTES = 2_000_000;
export const MAX_SWL_ROWS = 20_000;
export type SwlEntry = {
  hz: number; name: string; country: string; language: string; target: string;
  time: string; days: string; startMinute: number; endMinute: number;
  dayMask: number | null; dateStart: number | null; dateEnd: number | null;
  datesUnknown: boolean;
};
export type SwlCatalogue = { entries: readonly SwlEntry[]; rejected: number };

const DAY_NAMES = ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su'];
// A null mask is an irregular/tentative/special schedule, not "on air now".
export function swlDayMask(raw: string): number | null {
  const input = raw.trim();
  if (!input || input === 'daily') return 127;
  if (/^[1-7]+$/.test(input)) return [...input].reduce((mask, n) => mask | 1 << (Number(n) - 1), 0);
  let mask = 0;
  for (const part of input.split(',')) {
    const range = /^(Mo|Tu|We|Th|Fr|Sa|Su)-(Mo|Tu|We|Th|Fr|Sa|Su)$/.exec(part);
    if (range) {
      const end = DAY_NAMES.indexOf(range[2]!);
      for (let day = DAY_NAMES.indexOf(range[1]!); ; day = (day + 1) % 7) {
        mask |= 1 << day;
        if (day === end) break;
      }
    } else {
      const names = part.match(/Mo|Tu|We|Th|Fr|Sa|Su/g);
      if (!names || names.join('') !== part) return null;
      for (const name of names) mask |= 1 << DAY_NAMES.indexOf(name);
    }
  }
  return mask || null;
}
function minute(raw: string, end: boolean): number | null {
  if (!/^\d{4}$/.test(raw)) return null;
  if (end && raw === '2400') return 1440;
  const hours = Number(raw.slice(0, 2)), minutes = Number(raw.slice(2));
  return hours < 24 && minutes < 60 ? hours * 60 + minutes : null;
}
function dateCode(raw: string): number | null {
  if (!/^\d{4}$/.test(raw)) return null; // [0923] is a log date, not validity.
  const day = Number(raw.slice(0, 2)), month = Number(raw.slice(2));
  if (month < 1 || month > 12 || day < 1 || day > new Date(Date.UTC(2000, month, 0)).getUTCDate()) return null;
  return month * 100 + day;
}
function fields(line: string): string[] | null {
  const output: string[] = []; let value = '', quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') {
      if (quoted && line[i + 1] === '"') { value += '"'; i++; }
      else quoted = !quoted;
    } else if (c === ';' && !quoted) { output.push(value.trim()); value = ''; }
    else value += c;
  }
  if (quoted) return null;
  output.push(value.trim()); return output;
}
function clean(raw: string, limit: number): string {
  return raw.replace(/[\x00-\x1f\x7f]/g, ' ').trim().slice(0, limit);
}
export function decodeSwlCsv(bytes: ArrayBuffer): string {
  if (bytes.byteLength > MAX_SWL_BYTES) throw Error('SWL CSV is larger than 2 MB');
  try { return new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
  catch { return new TextDecoder('windows-1252').decode(bytes); }
}

export async function readSwlResponse(response: Response): Promise<ArrayBuffer> {
  if (Number(response.headers.get('content-length')) > MAX_SWL_BYTES) throw Error('SWL CSV is larger than 2 MB');
  if (!response.body) throw Error('Empty SWL CSV response');
  const reader = response.body.getReader(); const chunks: Uint8Array[] = []; let bytes = 0;
  try {
    for (;;) {
      const part = await reader.read();
      if (part.done) break;
      bytes += part.value.byteLength;
      if (bytes > MAX_SWL_BYTES) { await reader.cancel(); throw Error('SWL CSV is larger than 2 MB'); }
      chunks.push(part.value);
    }
  } finally { reader.releaseLock(); }
  const result = new Uint8Array(bytes); let offset = 0;
  for (const chunk of chunks) { result.set(chunk, offset); offset += chunk.byteLength; }
  return result.buffer;
}

/** Parse once, yielding between small batches; never runs in the FFT/audio path. */
export async function parseSwlCsv(
  text: string, yieldTask: () => Promise<void> = () => new Promise(done => setTimeout(done, 0)),
): Promise<SwlCatalogue> {
  if (text.length > MAX_SWL_BYTES) throw Error('SWL CSV is larger than 2 MB');
  const lines = text.replace(/^\uFEFF/, '').split(/\r?\n/);
  if (lines.length > MAX_SWL_ROWS + 2) throw Error('SWL CSV contains too many rows');
  const entries: SwlEntry[] = []; let rejected = 0;
  for (let i = 0; i < lines.length; i++) {
    if (i && i % 256 === 0) await yieldTask();
    const line = lines[i]!.trim();
    if (!line || line.startsWith('#') || /^kHz(?::|;)/i.test(line)) continue;
    const row = fields(line);
    if (!row || row.length < 7 || !/^\d+(?:\.\d+)?$/.test(row[0]!)) { rejected++; continue; }
    const hz = Math.round(Number(row[0]) * 1000), time = row[1]!, parts = time.split('-');
    const start = minute(parts[0] || '', false), end = minute(parts[1] || '', true);
    const persistence = Number(row[8] || '0');
    if (!Number.isSafeInteger(hz) || hz <= 0 || hz > 2_000_000_000 || parts.length !== 2
      || start === null || end === null || start === end || !row[4] || persistence === 8 || persistence === 98) {
      rejected++; continue;
    }
    const datesApply = persistence === 6 || persistence === 96;
    const dateStart = datesApply ? dateCode(row[9] || '') : null;
    const dateEnd = datesApply ? dateCode(row[10] || '') : null;
    entries.push({ hz, name: clean(row[4], 120), country: clean(row[3]!, 24),
      language: clean(row[5]!, 24), target: clean(row[6]!, 32), time, days: clean(row[2]!, 24),
      startMinute: start, endMinute: end, dayMask: swlDayMask(row[2]!), dateStart, dateEnd,
      datesUnknown: datesApply && (dateStart === null || dateEnd === null) });
  }
  if (!entries.length) throw Error('No usable station schedules found in SWL CSV');
  entries.sort((a, b) => a.hz - b.hz || a.name.localeCompare(b.name));
  return { entries, rejected };
}

export function swlScheduledAt(entry: SwlEntry, now: Date): boolean {
  if (entry.dayMask === null || entry.datesUnknown || !Number.isFinite(now.getTime())) return false;
  const current = now.getUTCHours() * 60 + now.getUTCMinutes();
  const overnight = entry.endMinute < entry.startMinute;
  const allDay = entry.startMinute === entry.endMinute;
  if (!allDay && !(overnight ? current >= entry.startMinute || current < entry.endMinute
    : current >= entry.startMinute && current < entry.endMinute)) return false;
  // Overnight broadcasts belong to the UTC day on which they start.
  const anchor = new Date(now.getTime() - (overnight && current < entry.endMinute ? 86_400_000 : 0));
  const day = (anchor.getUTCDay() + 6) % 7;
  if (!(entry.dayMask & 1 << day)) return false;
  if (entry.dateStart !== null && entry.dateEnd !== null) {
    const date = (anchor.getUTCMonth() + 1) * 100 + anchor.getUTCDate();
    if (entry.dateStart <= entry.dateEnd ? date < entry.dateStart || date > entry.dateEnd
      : date < entry.dateStart && date > entry.dateEnd) return false;
  }
  return true;
}

export type SwlMarker = { hz: number; entries: SwlEntry[] };
/** Binary frequency lookup; no scan of the complete catalogue on each draw. */
export function swlMarkersInView(
  catalogue: SwlCatalogue, low: number, high: number, now: Date, scheduledOnly: boolean,
): SwlMarker[] {
  if (!Number.isFinite(low) || !Number.isFinite(high) || high <= low) return [];
  const rows = catalogue.entries;
  let left = 0, right = rows.length;
  while (left < right) { const mid = (left + right) >>> 1; if (rows[mid]!.hz < low) left = mid + 1; else right = mid; }
  const markers: SwlMarker[] = [];
  for (let i = left; i < rows.length && rows[i]!.hz <= high; i++) {
    const entry = rows[i]!;
    if (scheduledOnly && !swlScheduledAt(entry, now)) continue;
    let marker = markers.at(-1);
    if (!marker || marker.hz !== entry.hz) { marker = { hz: entry.hz, entries: [] }; markers.push(marker); }
    marker.entries.push(entry);
  }
  return markers;
}

export type StationLabel = SwlMarker & { x: number; left: number; width: number; lane: number; label: string };
export function layoutSwlLabels(markers: readonly SwlMarker[], low: number, high: number, width: number): StationLabel[] {
  if (!Number.isFinite(width) || width < 80 || high <= low) return [];
  const center = (low + high) / 2;
  // Keep the tuned neighborhood when more candidates exist than can be drawn.
  const bounded = [...markers].sort((a, b) => Math.abs(a.hz - center) - Math.abs(b.hz - center))
    .slice(0, 96).sort((a, b) => a.hz - b.hz);
  const ends = [-4, -4]; let labels = 0;
  return bounded.map(marker => {
    const names = [...new Set(marker.entries.map(e => e.name))];
    const label = `${names[0]}${names.length > 1 ? ` +${names.length - 1}` : ''}`;
    const x = (marker.hz - low) / (high - low) * width;
    const labelWidth = Math.min(width, 176, Math.max(64, label.length * 6.5 + 12));
    const labelLeft = Math.max(0, Math.min(width - labelWidth, x - labelWidth / 2));
    const lane = labels >= 24 ? -1 : ends.findIndex(end => labelLeft >= end + 4);
    if (lane >= 0) { ends[lane] = labelLeft + labelWidth; labels++; }
    return { ...marker, x, left: labelLeft, width: labelWidth, lane, label };
  });
}
