import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { HAM_SEGMENTS, visibleBandSegments } from '../src/radio/band-plan';
import { swlDayMask, parseSwlCsv, readSwlResponse, decodeSwlCsv, swlScheduledAt, swlMarkersInView, layoutSwlLabels,
  MAX_SWL_BYTES, MAX_SWL_ROWS } from '../src/radio/swl-schedule';
import { createAppState } from '../src/state/app-state';
import { applyDisplayPrefsToState } from '../src/state/apply-prefs';
import { displayPrefsFromState } from '../src/state/prefs-from-state';
import { normalizeBandMemory, normalizeDisplayPrefs } from '../src/settings/normalize';
import { bandKeyForFrequency, bandEdgesForPanel, SWL_BAND_EDGES_HZ, HAM_BAND_EDGES_HZ } from '../src/radio/band';

const noYield = async () => {};
const csv = (frequency: string, time = '0000-2400', days = '', name = 'Test', persistence = '1', start = '', end = '') =>
  `${frequency};${time};${days};USA;${name};E;NAm;;${persistence};${start};${end}`;

describe('display-only band guidance', () => {
  it('provides 27 MHz and FM edges in the SWL panel without changing broadcast memories', () => {
    const edges = bandEdgesForPanel('swl');
    expect(edges.find(b => b.label === '11m')).toEqual(HAM_BAND_EDGES_HZ.find(b => b.label === '11m'));
    expect(edges.find(b => b.label === 'FM')).toEqual(HAM_BAND_EDGES_HZ.find(b => b.label === 'FM'));
    expect(edges.find(b => b.label === 'SWL 11m')).toEqual(SWL_BAND_EDGES_HZ.find(b => b.label === 'SWL 11m'));
    expect(bandKeyForFrequency(27_385_000, 'swl')).toBe('11m');
    expect(bandKeyForFrequency(25_900_000, 'swl')).toBe('SWL 11m');
    expect(bandEdgesForPanel('ham')).toBe(HAM_BAND_EDGES_HZ);
    const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
    expect(template).toContain('const HAM_BAND_EDGES_HZ = _next.HAM_BAND_EDGES_HZ;');
    expect(template.match(/_next.bandEdgesForPanel\(state.bandPanelGroup\)/g)).toHaveLength(2);
  });
  it('shows the US 40m data/phone boundary and clips exactly to the viewport', () => {
    const segments = visibleBandSegments(HAM_SEGMENTS, 7_100_000, 7_150_000);
    expect(segments.map(s => [s.label, s.left, s.width])).toEqual([
      ['Data / CW', 0, 50], ['Phone / CW', 50, 50],
    ]);
    expect(visibleBandSegments(HAM_SEGMENTS, NaN, 7e6)).toEqual([]);
    expect(visibleBandSegments(HAM_SEGMENTS, 8e6, 7e6)).toEqual([]);
  });
  it('uses real 60m channel widths and leaves gaps instead of painting one allocation', () => {
    const segments = HAM_SEGMENTS.filter(s => s.band === '60m');
    expect(segments.map(s => s.end - s.start)).toEqual([2800, 2800, 15000, 2800, 2800]);
    expect(visibleBandSegments(segments, 5_350_000, 5_351_000)).toEqual([]);
    expect(segments[2]!.start).toBe(5_351_500);
    expect(HAM_SEGMENTS.some(s => s.start <= 27_385_000 && s.end > 27_385_000)).toBe(false);
    // Guidance must not relabel/alter the existing 11m operating memory.
    expect(bandKeyForFrequency(27_385_000)).toBe('11m');
  });
  it('keeps valid ordered, nonoverlapping segments within each ham band', () => {
    for (let i = 0; i < HAM_SEGMENTS.length; i++) {
      const s = HAM_SEGMENTS[i]!;
      expect(s.start).toBeLessThan(s.end);
      const previous = HAM_SEGMENTS[i - 1];
      if (previous?.band === s.band) expect(previous.end).toBeLessThanOrEqual(s.start);
    }
  });
  it('round-trips the new options through per-band memories, preserving partial updates', () => {
    const state = createAppState();
    applyDisplayPrefsToState({ showBandSegments: false, showSwlStations: false, swlScheduleNowOnly: false }, state);
    applyDisplayPrefsToState({ waterfallContrast: 120 }, state);
    const memories = normalizeBandMemory({ 'SWL 31m': { frequency: 9_600_000, displayZoom: 5,
      radioPrefs: {}, displayPrefs: displayPrefsFromState(state) } });
    const restored = createAppState();
    applyDisplayPrefsToState(memories['SWL 31m']!.displayPrefs, restored);
    expect([restored.showBandSegments, restored.showSwlStations, restored.swlScheduleNowOnly]).toEqual([false, false, false]);
    expect(normalizeDisplayPrefs({}).showBandSegments).toBe(true);
    expect(normalizeDisplayPrefs({}).showSwlStations).toBe(true);
  });
});

describe('EiBi/Thetis CSV and UTC schedules', () => {
  it('bounds a streamed response even without a trustworthy content-length', async () => {
    let cancelled = false;
    const response = new Response(new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array(MAX_SWL_BYTES)); controller.enqueue(new Uint8Array(1));
      }, cancel() { cancelled = true; },
    }));
    await expect(readSwlResponse(response)).rejects.toThrow('2 MB');
    expect(cancelled).toBe(true);
    const text = csv('6000');
    expect(decodeSwlCsv(await readSwlResponse(new Response(text)))).toBe(text);
    await expect(readSwlResponse(new Response('x', { headers: { 'content-length': String(MAX_SWL_BYTES + 1) } }))).rejects.toThrow('2 MB');
  });
  it('handles quoted semicolons, decimal kHz, LF/CRLF, a header and UTF-8 BOM', async () => {
    let yields = 0;
    const input = '\uFEFFkHz:75;Time(UTC):93;Days:59;ITU:49;Station:201;Lng:49;Target:62;Remarks:135;P:35;Start:60;Stop:60;\r\n'
      + csv('7850.25', '1000-1100', 'Mo-Fr', '"Name; with ""quotes"""') + '\r\n' + csv('5000') + '\n';
    const catalogue = await parseSwlCsv(input, async () => { yields++; });
    expect(catalogue.entries.map(s => [s.hz, s.name])).toEqual([[5_000_000, 'Test'], [7_850_250, 'Name; with "quotes"']]);
    expect(catalogue.rejected).toBe(0);
    expect(yields).toBe(0);
    expect(decodeSwlCsv(Uint8Array.from([0x63, 0x61, 0x66, 0xe9]).buffer)).toBe('café');
    expect(decodeSwlCsv(new TextEncoder().encode('café').buffer)).toBe('café');
  });
  it('rejects malformed and inactive entries and caps allocation', async () => {
    const text = [csv('5000'), csv('NaN'), csv('Infinity'), csv('-1'), csv('5000', '2500-2600'),
      csv('5000', '1000-1000'), csv('5000', '0000-2400', '', 'Inactive', '8'), '5000;bad',
      csv('5000', '0000-2400', '', '"unfinished')].join('\n');
    const catalogue = await parseSwlCsv(text, noYield);
    expect(catalogue.entries).toHaveLength(1); expect(catalogue.rejected).toBe(8);
    await expect(parseSwlCsv('x'.repeat(MAX_SWL_BYTES + 1), noYield)).rejects.toThrow('2 MB');
    await expect(parseSwlCsv('\n'.repeat(MAX_SWL_ROWS + 3), noYield)).rejects.toThrow('too many');
  });
  it('yields cooperatively and accepts the complete current source snapshot', async () => {
    const text = readFileSync(new URL('../../templates/saturn-swl.csv', import.meta.url), 'utf8');
    let yields = 0;
    const catalogue = await parseSwlCsv(text, async () => { yields++; });
    expect(catalogue.entries.length).toBeGreaterThan(8500);
    expect(catalogue.entries.length + catalogue.rejected).toBe(8971);
    expect(yields).toBeGreaterThan(30);
    expect(catalogue.entries.every((e, i, a) => !i || a[i - 1]!.hz <= e.hz)).toBe(true);
  });
  it('handles lists, numeric days and ranges crossing Sunday; irregular entries are unknown', () => {
    expect(swlDayMask('')).toBe(127); expect(swlDayMask('Mo-Fr')).toBe(31);
    expect(swlDayMask('SaSu')).toBe(96); expect(swlDayMask('We-Mo')).toBe(125);
    expect(swlDayMask('Tu,Th')).toBe(10); expect(swlDayMask('135')).toBe(21);
    for (const days of ['irr', 'alt', 'tent', '1.Sa', 'Last7', 'MF-15']) expect(swlDayMask(days)).toBeNull();
  });
  it('uses UTC rather than local time, exclusive stop and the start day for overnight schedules', async () => {
    const [entry] = (await parseSwlCsv(csv('6000', '2300-0100', 'Mo'), noYield)).entries;
    expect(swlScheduledAt(entry!, new Date('2026-11-16T23:00:00Z'))).toBe(true);
    expect(swlScheduledAt(entry!, new Date('2026-11-17T00:59:00Z'))).toBe(true);
    expect(swlScheduledAt(entry!, new Date('2026-11-17T01:00:00Z'))).toBe(false);
    expect(swlScheduledAt(entry!, new Date('2026-11-17T23:30:00Z'))).toBe(false);
    expect(swlScheduledAt(entry!, new Date('invalid'))).toBe(false);
  });
  it('respects DDMM validity across New Year and does not mistake last-heard brackets for dates', async () => {
    const entries = (await parseSwlCsv([csv('6000', '0000-2400', '', 'Seasonal', '6', '0111', '2802'),
      csv('6001', '0000-2400', '', 'Last heard', '1', '', '[0923]'),
      csv('6002', '0000-2400', '', 'Unknown dates', '6', '', '')].join('\n'), noYield)).entries;
    expect(swlScheduledAt(entries[0]!, new Date('2026-11-17T12:00:00Z'))).toBe(true);
    expect(swlScheduledAt(entries[0]!, new Date('2027-01-03T12:00:00Z'))).toBe(true);
    expect(swlScheduledAt(entries[0]!, new Date('2027-03-03T12:00:00Z'))).toBe(false);
    expect(swlScheduledAt(entries[1]!, new Date('2026-11-17T12:00:00Z'))).toBe(true);
    expect(swlScheduledAt(entries[2]!, new Date('2026-11-17T12:00:00Z'))).toBe(false);
  });
  it('filters a visible frequency slice and merges schedules on the same carrier', async () => {
    const catalogue = await parseSwlCsv([csv('100'), csv('6000'), csv('6000', '1200-1300', '', 'Other'),
      csv('6100', '1200-1300', 'irr'), csv('6200', '1300-1400'), csv('30000')].join('\n'), noYield);
    const now = new Date('2026-11-17T12:00:00Z');
    expect(swlMarkersInView(catalogue, 6e6, 6.2e6, now, true).map(s => [s.hz, s.entries.length])).toEqual([[6e6, 2]]);
    expect(swlMarkersInView(catalogue, 6e6, 6.2e6, now, false)).toHaveLength(3);
    expect(swlMarkersInView(catalogue, 7e6, 6e6, now, false)).toEqual([]);
  });
  it('bounds crowded display work and never overlaps labels on the same lane or leaves the viewport', async () => {
    const catalogue = await parseSwlCsv(Array.from({ length: 1000 }, (_, i) => csv(String(6000 + i), '0000-2400', '', 'A long station name')).join('\n'), noYield);
    const markers = swlMarkersInView(catalogue, 6e6, 7e6, new Date('2026-11-17T12:00:00Z'), true);
    const layout = layoutSwlLabels(markers, 6e6, 7e6, 360);
    expect(layout).toHaveLength(96);
    expect(layout.filter(x => x.lane >= 0).length).toBeLessThanOrEqual(24);
    for (const lane of [0, 1]) {
      const labels = layout.filter(x => x.lane === lane);
      labels.forEach((label, i) => {
        expect(label.left).toBeGreaterThanOrEqual(0);
        expect(label.left + label.width).toBeLessThanOrEqual(360);
        if (i) expect(label.left).toBeGreaterThanOrEqual(labels[i - 1]!.left + labels[i - 1]!.width + 4);
      });
    }
  });
});
