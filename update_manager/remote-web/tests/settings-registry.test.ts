import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  SAFETY_PINNED_IDS,
  SETTINGS_REGISTRY,
  SETTINGS_REGISTRY_BY_ID,
  SETTINGS_SECTIONS,
  type SettingsSectionId,
} from '../src/settings/registry';
import { NON_SETTING_CONTROL_IDS, NON_SETTING_CONTROLS } from '../src/settings/registry-exclusions';
import {
  ANONYMOUS_DISCOVERY_RESIDUALS,
  ANONYMOUS_DISCOVERY_ROWS,
  resolveAnonymousDiscoveryRow,
} from '../src/settings/discovery-adapter';
import { searchSettings } from '../src/settings/search';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);
const templateIds = new Set(
  // Require a real attribute boundary: `data-setup-panel-id="display"` must not
  // count as an element with id="display".
  Array.from(template.matchAll(/(?:^|\s)id="([^"]+)"/g)).map((match) => String(match[1])),
);
const interactiveIds = Array.from(
  template.matchAll(/<(?:button|select|input|summary|textarea)\b[^>]*?(?:^|\s)id="([^"]+)"/gi),
).map((match) => String(match[1]));

/** Text of the element carrying `id`, used for the semantic target check. */
function elementText(id: string): string {
  const match = new RegExp(`<(?:button|select|input|summary|textarea|div|span|section)\\b[^>]*?(?:^|\\s)id="${id}"`, 'i').exec(template);
  const index = match ? match.index : -1;
  if (index < 0) return '';
  const open = index;
  const close = template.indexOf('>', index);
  const after = template.slice(close + 1);
  const text = after.slice(0, after.indexOf('</') < 0 ? 160 : after.indexOf('</'));
  return `${template.slice(open, close)} ${text}`;
}

/** Element tag carrying `id`, for stable shape assertions (no live text). */
function elementTag(id: string): string {
  const match = new RegExp(`<(\\w+)[^>]*?(?:^|\\s)id="${id}"`, 'i').exec(template);
  return match ? String(match[1]).toLowerCase() : '';
}

// Corrected post-states. Each pins the target, the element shape, and the wrong
// target it must never return to, so a no-op edit cannot pass unnoticed.
const CORRECTED_TARGETS: Record<string, { targetId: string; tag: string; notTargetId: string }> = {
  'transmit.pairNative': { targetId: 'satp-pair-btn', tag: 'button', notTargetId: 'tx-audio-profile-status' },
  'display.span': { targetId: 'sample-rate-readout', tag: 'span', notTargetId: 'display-resolution' },
  'display.history': { targetId: 'terrain-status', tag: 'span', notTargetId: 'display-caption' },
  'meter.analog': { targetId: 'meter-analog-option', tag: 'details', notTargetId: 'multimeter-face' },
  'meter.details': { targetId: 'meter-details-disclosure', tag: 'details', notTargetId: 'instrument-meter-deck' },
  'meter.dbfsBars': { targetId: 'instrument-rx-audio-meters', tag: 'div', notTargetId: 'audio-buffer-meter' },
};

// Verified inventories from the Phase 0 audit (message 107).
const QUERY_PARAMETERS = [
  'display_profile', 'rx_audio_adaptive', 'rx_audio_codec', 'tx_cfc', 'tx_cfc_precomp_db',
  'tx_diagnostic', 'tx_level_db', 'tx_noise_gate', 'tx_noise_gate_db', 'tx_opus', 'tx_source',
  'tx_voice_eq', 'transport', 'phase42_split', 'force_legacy_ws',
];
const STORAGE_KEYS = [
  'saturn.remote.activeProfile', 'saturn.remote.audioInputDeviceId', 'saturn.remote.audioOutputDeviceId',
  'saturn.remote.bandMemory', 'saturn.remote.displayPrefs', 'saturn.remote.freqLock',
  'saturn.remote.instrumentMeterMode', 'saturn.remote.keepScreenAwake', 'saturn.remote.operationsDrawer',
  'saturn.remote.phoneSpectrumMode', 'saturn.remote.phoneWaterfall', 'saturn.remote.radioPrefs',
  'saturn.remote.setupPanel', 'saturn.remote.spectrumWaterfallRatio', 'saturn.remote.vfoTuneStepHz',
];
const SECTION_NINE_TARGETS = [
  'operator-conn-pill', 'shortcut-help-btn', 'layout-btn', 'theme-btn', 'header-setup-btn',
  'operations-tab-memory', 'operations-tab-audio', 'operations-tab-network', 'operations-tab-dsp',
  'operations-tab-radio', 'operations-tab-log',
  'setup-tx-puresignal-enabled', 'setup-tx-phase-rotator-enabled', 'setup-tx-two-tone-section',
  'setup-tx-timeout-enabled', 'tx-dexp-enabled', 'tx-speech-processor-enabled', 'tx-cessb-enabled',
  'tx-noise-gate-enabled', 'tx-eq-enable-btn', 'freq-entry-overlay', 'operations-drawer',
  'terrain-diagnostics-enabled', 'setup-network-copy-diagnostics-btn',
];

describe('settings registry schema', () => {
  it('has unique, well-formed entries', () => {
    const ids = SETTINGS_REGISTRY.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const entry of SETTINGS_REGISTRY) {
      expect(entry.id).toMatch(/^[a-z0-9]+(\.[A-Za-z0-9]+)+$/);
      expect(entry.label.trim().length).toBeGreaterThan(0);
      expect(entry.description.trim().length).toBeGreaterThan(0);
      expect(entry.targetId.trim().length).toBeGreaterThan(0);
      expect(SETTINGS_SECTIONS.some((section) => section.id === entry.section)).toBe(true);
    }
  });

  it('leaves no section empty and no duplicate labels within a section', () => {
    for (const section of SETTINGS_SECTIONS) {
      const entries = SETTINGS_REGISTRY.filter((entry) => entry.section === section.id);
      expect(entries.length, `section ${section.id} is empty`).toBeGreaterThan(0);
      const labels = entries.map((entry) => entry.label.toLowerCase());
      expect(new Set(labels).size, `duplicate label in ${section.id}`).toBe(labels.length);
    }
  });

  it('points every DOM target at an element that exists in the template', () => {
    const domKinds = ['control', 'readout', 'toggle', 'select', 'slider', 'button', 'indicator', 'overlay'];
    for (const entry of SETTINGS_REGISTRY) {
      if (!domKinds.includes(entry.targetKind)) continue;
      expect(templateIds.has(entry.targetId), `${entry.id} -> #${entry.targetId} is missing`).toBe(true);
    }
  });

  it('covers every verified query parameter and storage key', () => {
    const targets = new Set(SETTINGS_REGISTRY.map((entry) => entry.targetId));
    for (const parameter of QUERY_PARAMETERS) {
      expect(targets.has(parameter), `query parameter ${parameter} is not indexed`).toBe(true);
    }
    for (const key of STORAGE_KEYS) {
      expect(targets.has(key), `storage key ${key} is not indexed`).toBe(true);
    }
  });

  it('covers the section 9 checklist targets', () => {
    const targets = new Set(SETTINGS_REGISTRY.map((entry) => entry.targetId));
    for (const target of SECTION_NINE_TARGETS) {
      expect(targets.has(target), `section 9 control ${target} is not indexed`).toBe(true);
    }
  });

  it('marks the safety-pinned items that may never be unpinned', () => {
    for (const id of SAFETY_PINNED_IDS) {
      const entry = SETTINGS_REGISTRY_BY_ID.get(id);
      expect(entry, `${id} is not in the registry`).toBeDefined();
      expect(entry?.safetyPinned).toBe(true);
    }
  });

  it('flags radio-state sections so restore-defaults can avoid them', () => {
    const radioSections: SettingsSectionId[] = ['receive', 'transmit', 'radio'];
    for (const section of radioSections) {
      expect(SETTINGS_SECTIONS.find((item) => item.id === section)?.affectsRadio).toBe(true);
    }
  });

  it('checks high-risk targets semantically, not just by id existence', () => {
    const highRisk = SETTINGS_REGISTRY.filter((entry) => entry.domTextToken);
    expect(highRisk.length).toBeGreaterThanOrEqual(7);
    for (const entry of highRisk) {
      const text = elementText(entry.targetId);
      expect(text.length, `no markup found for #${entry.targetId}`).toBeGreaterThan(0);
      expect(
        text.toLowerCase().includes(String(entry.domTextToken).toLowerCase()),
        `#${entry.targetId} does not look like "${entry.label}" (expected token "${entry.domTextToken}")`,
      ).toBe(true);
    }
  });

  it('pins every corrected target so a no-op edit cannot pass unnoticed', () => {
    for (const [id, expected] of Object.entries(CORRECTED_TARGETS)) {
      const entry = SETTINGS_REGISTRY_BY_ID.get(id);
      expect(entry, `${id} is missing from the registry`).toBeDefined();
      expect(entry?.targetId, `${id} target regressed`).toBe(expected.targetId);
      expect(entry?.targetId, `${id} points back at the old target`).not.toBe(expected.notTargetId);
      expect(elementTag(expected.targetId), `${id} element shape changed`).toBe(expected.tag);
    }
  });

  it('classifies every interactive control as indexed or explicitly excluded', () => {
    const indexed = new Set(SETTINGS_REGISTRY.map((entry) => entry.targetId));
    const unclassified = interactiveIds.filter(
      (id) => !indexed.has(id) && !NON_SETTING_CONTROL_IDS.has(id),
    );
    expect(unclassified, `unclassified controls: ${unclassified.join(', ')}`).toEqual([]);
  });

  it('keeps the exclusion list honest: reasons, coverage and no stale ids', () => {
    const reasons = new Set(['navigation', 'duplicate', 'child-of-indexed']);
    const indexed = new Set(SETTINGS_REGISTRY.map((entry) => entry.targetId));
    for (const exclusion of NON_SETTING_CONTROLS) {
      expect(reasons.has(exclusion.reason), `bad reason for ${exclusion.id}`).toBe(true);
      expect(exclusion.note.trim().length, `${exclusion.id} needs a note`).toBeGreaterThan(0);
      expect(templateIds.has(exclusion.id), `${exclusion.id} no longer exists`).toBe(true);
      expect(indexed.has(exclusion.id), `${exclusion.id} is indexed and excluded`).toBe(false);
      if (exclusion.coveredBy) {
        expect(SETTINGS_REGISTRY_BY_ID.has(exclusion.coveredBy), `${exclusion.id} points at unknown ${exclusion.coveredBy}`).toBe(true);
      }
    }
  });
});

describe('settings search', () => {
  it('finds an entry by its new label', () => {
    expect(searchSettings('receive volume')[0]?.id).toBe('receive.volume');
  });

  it('finds controls by their pre-redesign names', () => {
    expect(searchSettings('SQL').some((entry) => entry.id.startsWith('receive.voiceSquelch'))).toBe(true);
    expect(searchSettings('MON').some((entry) => entry.id === 'transmit.monitor')).toBe(true);
    expect(searchSettings('ATT').some((entry) => entry.id === 'radio.attenuator')).toBe(true);
    expect(searchSettings('SATP').some((entry) => entry.id === 'transmit.pairNative')).toBe(true);
  });

  it('is case-insensitive and matches description words', () => {
    expect(searchSettings('ATTENUATOR').some((entry) => entry.id === 'radio.attenuator')).toBe(true);
    expect(searchSettings('push to talk').some((entry) => entry.id === 'transmit.ptt')).toBe(true);
  });

  it('scopes search to a section and honours the limit', () => {
    const transmitOnly = searchSettings('a', { section: 'transmit' });
    expect(transmitOnly.every((entry) => entry.section === 'transmit')).toBe(true);
    expect(searchSettings('', { limit: 5 }).length).toBe(5);
  });

  it('returns nothing for a query that matches no entry', () => {
    expect(searchSettings('zzzz-no-such-setting')).toEqual([]);
  });

  it('finds every entry by its own label and by each of its synonyms', () => {
    for (const entry of SETTINGS_REGISTRY) {
      expect(searchSettings(entry.label).some((found) => found.id === entry.id), `label ${entry.label}`).toBe(true);
      for (const synonym of entry.synonyms) {
        expect(
          searchSettings(synonym).some((found) => found.id === entry.id),
          `synonym ${synonym} should find ${entry.id}`,
        ).toBe(true);
      }
    }
  });
});

describe('anonymous discovery rows', () => {
  it('accounts for every anonymous row the parity audit reported', () => {
    expect(ANONYMOUS_DISCOVERY_ROWS).toHaveLength(74);
  });

  it('gives each row a unique key and a real owning entry', () => {
    const keys = ANONYMOUS_DISCOVERY_ROWS.map((row) => row.discoveryKey);
    expect(new Set(keys).size).toBe(keys.length);
    for (const row of ANONYMOUS_DISCOVERY_ROWS) {
      expect(row.note.trim().length, `${row.discoveryKey} needs a note`).toBeGreaterThan(0);
      if (row.matchedEntryId === null) {
        expect(row.decisionReason?.trim().length ?? 0, `${row.discoveryKey} needs a decision reason`).toBeGreaterThan(0);
        continue;
      }
      expect(
        SETTINGS_REGISTRY_BY_ID.has(row.matchedEntryId),
        `${row.discoveryKey} points at unknown entry ${row.matchedEntryId}`,
      ).toBe(true);
    }
  });

  it('reports the residual count explicitly', () => {
    // Every anonymous row has a defensible owner, so the strict audit should have
    // nothing left to decide. If that changes, this number changes with it.
    expect(ANONYMOUS_DISCOVERY_RESIDUALS).toHaveLength(0);
  });

  it('resolves rows by key', () => {
    expect(resolveAnonymousDiscoveryRow('band.20m')?.matchedEntryId).toBe('radio.band');
    expect(resolveAnonymousDiscoveryRow('keypad.del')?.matchedEntryId).toBe('radio.keypad');
    expect(resolveAnonymousDiscoveryRow('no.such.row')).toBeUndefined();
  });
});
