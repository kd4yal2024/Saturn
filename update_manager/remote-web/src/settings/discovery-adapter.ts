import { SETTINGS_REGISTRY, type SettingsEntry } from './registry';

/**
 * Adapter for the `audit:ui-parity` script.
 *
 * Discovery rows are keyed by the registry id (`entry.id`) and matched to the
 * template through `targetId`; `sourceControlIds` lets an entry declare extra
 * element ids it also drives (for example a control that exists once per tier).
 * Nothing here duplicates discovery content — it is a join key plus the fields an
 * audit needs, so the two lists cannot drift apart.
 */
export type ParityAuditRow = {
  discoveryKey: string;
  section: string;
  label: string;
  targetId: string;
  targetKind: SettingsEntry['targetKind'];
  sourceControlIds: string[];
  mainScreen: SettingsEntry['mainScreen'];
  safetyPinned: boolean;
  parityStatus: SettingsEntry['parityStatus'];
  existingSetupPanel: SettingsEntry['existingSetupPanel'] | null;
};

export function toParityAuditRows(registry: readonly SettingsEntry[] = SETTINGS_REGISTRY): ParityAuditRow[] {
  return registry.map((entry) => ({
    discoveryKey: entry.id,
    section: entry.section,
    label: entry.label,
    targetId: entry.targetId,
    targetKind: entry.targetKind,
    sourceControlIds: entry.sourceControlIds ?? [],
    mainScreen: entry.mainScreen,
    safetyPinned: Boolean(entry.safetyPinned),
    parityStatus: entry.parityStatus,
    existingSetupPanel: entry.existingSetupPanel ?? null,
  }));
}

/** Entries that already live in one of the legacy setup panels. */
export function entriesWithSetupPanel(): ParityAuditRow[] {
  return toParityAuditRows().filter((row) => row.existingSetupPanel !== null);
}

/** Entries whose original control is outside the setup sheet and therefore need
 *  a deliberate mirror or navigation row rather than an in-sheet move. */
export function entriesOutsideSetupPanel(): ParityAuditRow[] {
  return toParityAuditRows().filter((row) => row.existingSetupPanel === null);
}

/**
 * Anonymous discovery rows: inventory lines whose control cell carries no
 * element id (segmented buttons, disclosure summaries, keypad keys), so the
 * audit cannot join them by id. Each is mapped to the registry entry that owns
 * the behaviour, with a note saying why that is the right owner.
 *
 * A row whose owner is genuinely undecided stays here with `matchedEntryId: null`
 * and a reason, so the residual count is visible instead of silently dropped.
 */
export type AnonymousDiscoveryMapping = {
  discoveryKey: string;
  matchedEntryId: string | null;
  note: string;
  /** Required when the row has no owner yet. */
  decisionReason?: string;
};

type LabelPair = readonly [string, string];

function group(prefix: string, pairs: readonly LabelPair[], entryId: string): AnonymousDiscoveryMapping[] {
  return pairs.map(([suffix, note]) => ({
    discoveryKey: `${prefix}.${suffix}`,
    matchedEntryId: entryId,
    note,
  }));
}

const BAND_LABELS: readonly LabelPair[] = [
  ['160m', '160 m band button'], ['80m', '80 m band button'], ['60m', '60 m band button'],
  ['40m', '40 m band button'], ['30m', '30 m band button'], ['20m', '20 m band button'],
  ['17m', '17 m band button'], ['15m', '15 m band button'], ['12m', '12 m band button'],
  ['10m', '10 m band button'], ['6m', '6 m band button'], ['FM', 'FM band button'],
];
const MODE_LABELS: readonly LabelPair[] = [
  ['USB', 'USB mode button'], ['LSB', 'LSB mode button'], ['AM', 'AM mode button'],
  ['SAM', 'SAM mode button'], ['FM', 'FM mode button'], ['DIGU', 'DIGU mode button'],
  ['DIGL', 'DIGL mode button'], ['CWU', 'CWU mode button'], ['CWL', 'CWL mode button'],
];
const DIGIT_LABELS: readonly LabelPair[] = [
  ['1', 'Keypad digit 1'], ['2', 'Keypad digit 2'], ['3', 'Keypad digit 3'],
  ['4', 'Keypad digit 4'], ['5', 'Keypad digit 5'], ['6', 'Keypad digit 6'],
  ['7', 'Keypad digit 7'], ['8', 'Keypad digit 8'], ['9', 'Keypad digit 9'],
  ['0', 'Keypad digit 0'], ['dot', 'Keypad decimal point'], ['del', 'Keypad delete'],
  ['clear', 'Keypad clear'],
];

export const ANONYMOUS_DISCOVERY_ROWS: readonly AnonymousDiscoveryMapping[] = [
  { discoveryKey: 'system.summary', matchedEntryId: 'network.diagnostics', note: 'Header System disclosure; hosts WS Probe, Copy Log, Saturn Go and Monitor.' },
  { discoveryKey: 'meter.details.summary', matchedEntryId: 'meter.details', note: 'More meter details and TX tools disclosure.' },
  { discoveryKey: 'txsetup.summary', matchedEntryId: 'transmit.txSetup', note: 'TX Setup disclosure: the whole gate, expander, processor, CESSB and filter card.' },
  { discoveryKey: 'passband.lower', matchedEntryId: 'receive.passband', note: 'Lower RX passband drag handle; the indexed entry covers the whole overlay including both handles.' },
  { discoveryKey: 'passband.upper', matchedEntryId: 'receive.passband', note: 'Upper RX passband drag handle.' },
  { discoveryKey: 'terrain.close', matchedEntryId: 'display.terrainSettings', note: 'Close button of the 3D settings panel.' },
  ...group('agc', [
    ['Off', 'AGC off segment'], ['Long', 'AGC long segment'], ['Slow', 'AGC slow segment'],
    ['Med', 'AGC medium segment'], ['Fast', 'AGC fast segment'],
  ], 'receive.agc'),
  ...group('nr', [
    ['Off', 'NR off segment'], ['NR1', 'NR1 segment'], ['NR2', 'NR2 segment'],
    ['NR3', 'NR3 segment'], ['NR4', 'NR4 segment'],
  ], 'receive.nr'),
  ...group('nb', [
    ['Off', 'NB off segment'], ['NB1', 'NB1 segment'], ['NB2', 'NB2 segment'], ['NB3', 'NB3 segment'],
  ], 'receive.nb'),
  ...group('band', BAND_LABELS, 'radio.band'),
  ...group('mode', MODE_LABELS, 'radio.mode'),
  ...group('sheet.band', BAND_LABELS, 'radio.band'),
  ...group('keypad', DIGIT_LABELS, 'radio.keypad'),
  ...group('keypad', [
    ['mhz', 'Keypad MHz unit'], ['khz', 'Keypad kHz unit'], ['hz', 'Keypad Hz unit'],
  ], 'radio.keypad'),
  ...group('phoneDock', [
    ['Radio', 'Phone dock Radio context'],
    ['RX', 'Phone dock RX context'],
    ['TX', 'Phone dock TX context'],
    ['DSP', 'Phone dock DSP context'],
    ['More', 'Phone dock overflow'],
  ], 'shell.mobileDock'),
];

export const ANONYMOUS_DISCOVERY_RESIDUALS: readonly AnonymousDiscoveryMapping[] =
  ANONYMOUS_DISCOVERY_ROWS.filter((row) => row.matchedEntryId === null);

export function resolveAnonymousDiscoveryRow(discoveryKey: string): AnonymousDiscoveryMapping | undefined {
  return ANONYMOUS_DISCOVERY_ROWS.find((row) => row.discoveryKey === discoveryKey);
}
