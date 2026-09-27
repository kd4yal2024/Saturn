#!/usr/bin/env node
// Generates docs/ui-redesign/migration.md from the settings registry, so the
// parity inventory cannot drift from the machine-readable index.
//
//   npm run generate:migration          # write the document
//   npm run check:migration             # fail if the committed document is stale
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  SAFETY_PINNED_IDS,
  SETTINGS_REGISTRY,
  SETTINGS_SECTIONS,
  type SettingsEntry,
} from '../src/settings/registry';
import { NON_SETTING_CONTROLS } from '../src/settings/registry-exclusions';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const updateManagerRoot = resolve(remoteWebRoot, '..');
const templatePath = resolve(updateManagerRoot, 'templates/saturn-remote-next.html');
const outputPath = resolve(updateManagerRoot, '../docs/ui-redesign/migration.md');

const template = readFileSync(templatePath, 'utf8');

// Interactive elements with ids, in document order, for the audit appendix.
const interactiveIds: string[] = [];
for (const match of template.matchAll(/<(button|select|input|summary|textarea)\b[^>]*\bid="([^"]+)"/gi)) {
  interactiveIds.push(String(match[2]));
}
const registeredTargets = new Set(SETTINGS_REGISTRY.map((entry) => entry.targetId));
const excludedIds = new Set(NON_SETTING_CONTROLS.map((entry) => entry.id));
const indexedInteractive = interactiveIds.filter((id) => registeredTargets.has(id));
const stillUnclassified = interactiveIds.filter(
  (id) => !registeredTargets.has(id) && !excludedIds.has(id),
);

function statusLabel(entry: SettingsEntry): string {
  if (entry.parityStatus === 'wired') return 'wired';
  if (entry.parityStatus === 'pending-integration') return 'pending integration';
  return 'no home yet';
}

function targetLabel(entry: SettingsEntry): string {
  if (entry.targetKind === 'query-param') return `?${entry.targetId}`;
  if (entry.targetKind === 'storage-key') return `localStorage \`${entry.targetId}\``;
  return `id \`${entry.targetId}\``;
}

function settingsLocation(entry: SettingsEntry): string {
  const section = SETTINGS_SECTIONS.find((item) => item.id === entry.section);
  const sectionLabel = section ? section.label : entry.section;
  if (entry.existingSetupPanel) return `${sectionLabel} → legacy ${entry.existingSetupPanel} panel`;
  return `${sectionLabel} → route to the original control`;
}

function mainLocation(entry: SettingsEntry): string {
  if (entry.safetyPinned) return 'main screen, always shown (locked)';
  if (entry.mainScreen === 'always') return 'main screen, default';
  if (entry.mainScreen === 'optional') return 'main screen, pinnable';
  return 'Settings only';
}

/** Evidence text: never claims a live-radio action was exercised by the offline fixture. */
function evidence(entry: SettingsEntry): string {
  if (entry.parityStatus === 'wired') {
    if (entry.targetKind === 'query-param' || entry.targetKind === 'storage-key') {
      return 'parameter documented; no DOM handler to exercise';
    }
    return 'Settings route covered by browser sweep; original handler retained. Live-radio action requires hardware QA';
  }
  if (entry.parityStatus === 'pending-integration') {
    return 'not verified by this author; Settings route is operative, per-control check outstanding';
  }
  return 'no home yet; decision outstanding';
}

const lines: string[] = [];
lines.push('# Saturn Remote redesign: control migration inventory');
lines.push('');
lines.push('Generated from `remote-web/src/settings/registry.ts` — do not hand-edit the tables.');
lines.push('Refresh with `npm run generate:migration`; CI can fail on drift with `npm run check:migration`.');
lines.push('');
lines.push('**How to read a row.** The registry is the machine-readable index of every control,');
lines.push('readout, indicator, overlay, query parameter and storage key. `Settings` is where the');
lines.push('entry lives in the new surface; `Main screen` is its default presence there.');
lines.push('');
lines.push('**Status legend.**');
lines.push('');
lines.push('- `wired` — the original control and handler remain, and its Settings route is covered');
lines.push('  by the browser registry sweep. The registry test fails if its target id disappears.');
lines.push('- `pending integration` — the feature exists but is only reachable through the legacy');
lines.push('  setup shell until Phase 3 wires the new surface. It is not claimed complete.');
lines.push('- `no home yet` — flagged for the owner or the integration owner rather than guessed.');
lines.push('');
lines.push('**Verification boundary.** Chromium checks Settings routes for DOM-backed entries at');
lines.push('phone and desktop widths, and focused probes exercise safe native controls and selected');
lines.push('original actions. Actions that require a live radio, transmit, or disconnection are');
lines.push('reserved for hardware QA; this offline evidence does not claim those actions ran.');
lines.push('');

for (const section of SETTINGS_SECTIONS) {
  const entries = SETTINGS_REGISTRY.filter((entry) => entry.section === section.id);
  lines.push(`## ${section.label} (${entries.length})`);
  lines.push('');
  lines.push(`${section.description}${section.affectsRadio ? ' — **live radio state**: no restore-defaults here.' : ''}`);
  lines.push('');
  lines.push('| Control | Current target | New main location | Settings location | Status | Evidence / remaining limitation |');
  lines.push('| --- | --- | --- | --- | --- | --- |');
  for (const entry of entries) {
    lines.push(`| ${entry.label} | ${targetLabel(entry)} | ${mainLocation(entry)} | ${settingsLocation(entry)} | ${statusLabel(entry)} | ${evidence(entry)} |`);
  }
  lines.push('');
}

lines.push('## Safety-pinned items that cannot be unpinned');
lines.push('');
for (const id of SAFETY_PINNED_IDS) {
  const entry = SETTINGS_REGISTRY.find((item) => item.id === id);
  lines.push(`- **${entry?.label ?? id}** — ${entry?.description ?? ''}`);
}
lines.push('');
lines.push('## Coverage');
lines.push('');
lines.push(`${interactiveIds.length} interactive elements carry an id in the template: **${indexedInteractive.length} indexed** and **${NON_SETTING_CONTROLS.length} explicitly excluded**, so nothing is unaccounted for.`);
lines.push('The registry test fails if a template control is neither indexed nor excluded, and also fails on a stale exclusion, so this total is enforced rather than asserted here.');
lines.push('');
if (stillUnclassified.length > 0) {
  lines.push('> **Unclassified controls remain:** ' + stillUnclassified.map((id) => `\`${id}\``).join(', '));
  lines.push('');
}
lines.push('## Excluded controls, with reasons');
lines.push('');
const reasonTitles: Record<string, string> = {
  navigation: 'Navigation and modal chrome',
  duplicate: 'Duplicates of an indexed control (per-tier or quick action)',
  'child-of-indexed': 'Parameters inside a control surface that is already indexed',
};
for (const reason of Object.keys(reasonTitles)) {
  const group = NON_SETTING_CONTROLS.filter((entry) => entry.reason === reason);
  if (group.length === 0) continue;
  lines.push(`### ${reasonTitles[reason]} (${group.length})`);
  lines.push('');
  lines.push('| Control | Covered by | Reason it is not a separate setting |');
  lines.push('| --- | --- | --- |');
  for (const exclusion of group) {
    const covered = exclusion.coveredBy
      ? (SETTINGS_REGISTRY.find((item) => item.id === exclusion.coveredBy)?.label ?? exclusion.coveredBy)
      : '—';
    lines.push(`| \`${exclusion.id}\` | ${covered} | ${exclusion.note} |`);
  }
  lines.push('');
}

const document = `${lines.join('\n')}`;

if (process.argv.includes('--check')) {
  let existing = '';
  try {
    existing = readFileSync(outputPath, 'utf8');
  } catch {
    console.error(`generate-migration-doc: ${outputPath} is missing`);
    process.exit(1);
  }
  if (existing !== document) {
    console.error('generate-migration-doc: migration.md is stale; run npm run generate:migration');
    process.exit(1);
  }
  console.log(`generate-migration-doc: OK (${SETTINGS_REGISTRY.length} entries, ${indexedInteractive.length} indexed controls, ${NON_SETTING_CONTROLS.length} excluded, ${stillUnclassified.length} unclassified)`);
} else {
  writeFileSync(outputPath, document);
  console.log(`generate-migration-doc: wrote ${outputPath} (${SETTINGS_REGISTRY.length} entries, ${indexedInteractive.length} indexed controls, ${NON_SETTING_CONTROLS.length} excluded, ${stillUnclassified.length} unclassified)`);
}
