#!/usr/bin/env node
// Compare every Phase 0 discovery row with the live Settings index, explicit
// exclusions, and the reviewed anonymous-row mappings.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const discoveryPath = resolve(root, '../../docs/ui-redesign/discovery.md');
const outputArg = process.argv.find(arg => arg.startsWith('--output='))?.slice(9);
const strict = process.argv.includes('--strict');

export function parseDiscoveryInventory(markdown) {
  const section = markdown.split('### Static interactive elements\n')[1]?.split('\n### ')[0];
  if (!section) throw new Error('Static interactive inventory section missing');
  const rows = [];
  for (const raw of section.split('\n')) {
    if (!/^\|\s*\d+\s*\|/.test(raw)) continue;
    // Discovery contains escaped pipes inside control labels; split only table
    // delimiters so handler areas and effects retain their original columns.
    const cells = raw.split(/(?<!\\)\|/).slice(1, -1).map(cell => cell.trim().replace(/\\\|/g, '|'));
    if (cells.length !== 5) throw new Error(`Malformed discovery table row: ${raw}`);
    const [sourceLine, location, control, handlerArea, effect] = cells;
    const ids = [...control.matchAll(/`#([A-Za-z][\w:-]*)`/g)].map(found => found[1]);
    rows.push({ key: ids.length ? `#${ids[0]}` : `line:${sourceLine}`,
      sourceLine: Number(sourceLine), location, control, handlerArea, effect, ids });
  }
  if (rows.length < 250) throw new Error(`Discovery inventory unexpectedly short: ${rows.length}`);
  rows.forEach((row, index) => {
    if (!row.ids.length) row.discoveryKey = anonymousDiscoveryKey(row, rows[index - 1], rows[index + 1]);
  });
  const keys = rows.map(row => row.key);
  const repeated = keys.filter((key, index) => keys.indexOf(key) !== index);
  if (repeated.length) throw new Error(`Discovery inventory keys repeated: ${[...new Set(repeated)].join(', ')}`);
  return rows;
}

function anonymousDiscoveryKey(row, previous, next) {
  const label = row.control.split(' — ').slice(1).join(' — ').trim();
  const adjacentLabel = item => item?.control.split(' — ').slice(1).join(' — ').trim();
  if (row.location === 'Header / System' && label === 'System') return 'system.summary';
  if (row.location === 'Meter / tools' && label.startsWith('More meter details')) return 'meter.details.summary';
  if (row.location === 'Transmit' && label.startsWith('TX Setup')) return 'txsetup.summary';
  if (row.location === 'Display toolbar/well') {
    if (label === 'Adjust lower RX passband edge') return 'passband.lower';
    if (label === 'Adjust upper RX passband edge') return 'passband.upper';
  }
  if (row.location === 'Terrain settings' && label === 'Close') return 'terrain.close';
  if (row.location === 'Receive / DSP') {
    const group = label === 'Off'
      ? ({ Long: 'agc', NR1: 'nr', NB1: 'nb' })[adjacentLabel(next)]
      : ['Long', 'Slow', 'Med', 'Fast'].includes(label) ? 'agc'
        : /^NR[1-4]$/.test(label) ? 'nr' : /^NB[1-3]$/.test(label) ? 'nb' : null;
    return group ? `${group}.${label}` : null;
  }
  if (row.location === 'Band / mode') {
    const group = /^\d+m$/.test(label) || (label === 'FM' && adjacentLabel(previous) === '6m')
      ? 'band' : ['USB', 'LSB', 'AM', 'SAM', 'FM', 'DIGU', 'DIGL', 'CWU', 'CWL'].includes(label)
        ? 'mode' : null;
    return group ? `${group}.${label}` : null;
  }
  if (row.location === 'Phone dock') return ['Radio', 'RX', 'TX', 'DSP', 'More'].includes(label)
    ? `phoneDock.${label}` : null;
  if (row.location === 'Frequency sheet') {
    if (/^\d+m$/.test(label) || label === 'FM') return `sheet.band.${label}`;
    const suffix = ({ '.': 'dot', Del: 'del', Clear: 'clear', MHz: 'mhz', kHz: 'khz', Hz: 'hz' })[label] || label;
    return /^[0-9]$/.test(label) || ['dot', 'del', 'clear', 'mhz', 'khz', 'hz'].includes(suffix)
      ? `keypad.${suffix}` : null;
  }
  return null;
}

async function loadSettingsClassification() {
  // Bundle TS in memory so the audit reads the same sources as the app/tests.
  const result = await build({ entryPoints: [resolve(root, 'src/settings/discovery-adapter.ts'),
    resolve(root, 'src/settings/registry-exclusions.ts')], bundle: true, platform: 'node',
    format: 'esm', write: false, outdir: 'audit-modules' });
  const modules = await Promise.all(result.outputFiles.map(file =>
    import(`data:text/javascript;base64,${Buffer.from(file.contents).toString('base64')}`)));
  const adapter = modules.find(module => typeof module.toParityAuditRows === 'function');
  const exclusions = modules.find(module => Array.isArray(module.NON_SETTING_CONTROLS));
  if (!adapter || !exclusions) throw new Error('Settings parity adapter or exclusions missing');
  if (!Array.isArray(adapter.ANONYMOUS_DISCOVERY_ROWS)) throw new Error('Anonymous discovery mapping missing');
  return { registryRows: adapter.toParityAuditRows(), exclusions: exclusions.NON_SETTING_CONTROLS,
    anonymousMappings: adapter.ANONYMOUS_DISCOVERY_ROWS };
}

export function auditDiscovery(rows, registryRows, exclusions, anonymousMappings) {
  const indexed = new Map();
  const registryKeys = new Set();
  const duplicateRegistryKeys = [];
  for (const entry of registryRows) {
    if (!entry || typeof entry.discoveryKey !== 'string' ||
      typeof entry.targetId !== 'string' || !Array.isArray(entry.sourceControlIds)) {
      throw new Error('Invalid Settings parity adapter row');
    }
    if (registryKeys.has(entry.discoveryKey)) duplicateRegistryKeys.push(entry.discoveryKey);
    registryKeys.add(entry.discoveryKey);
    const controlIds = ['query-param', 'storage-key'].includes(entry.targetKind)
      ? entry.sourceControlIds : [entry.targetId, ...entry.sourceControlIds];
    for (const id of controlIds) {
      if (typeof id !== 'string' || !id) throw new Error(`Invalid control id in ${entry.discoveryKey}`);
      const cleanId = id.replace(/^#/, '');
      if (!indexed.has(cleanId)) indexed.set(cleanId, []);
      indexed.get(cleanId).push(entry.discoveryKey);
    }
  }
  const excluded = new Map();
  const duplicateExclusionIds = [];
  const staleExclusionOwners = [];
  for (const item of exclusions) {
    if (!item || typeof item.id !== 'string' ||
      !['navigation', 'duplicate', 'child-of-indexed'].includes(item.reason) ||
      typeof item.note !== 'string' || !item.note.trim()) {
      throw new Error('Invalid explicit Settings exclusion');
    }
    if (excluded.has(item.id)) duplicateExclusionIds.push(item.id);
    excluded.set(item.id, item);
    if (item.coveredBy && !registryKeys.has(item.coveredBy)) staleExclusionOwners.push(item.id);
  }
  const idRows = rows.filter(row => row.ids.length);
  const mappingByKey = new Map();
  const duplicateAnonymousKeys = [];
  const invalidAnonymousOwners = [];
  for (const mapping of anonymousMappings) {
    if (!mapping || typeof mapping.discoveryKey !== 'string' ||
      typeof mapping.note !== 'string' || !mapping.note.trim()) {
      throw new Error('Invalid anonymous discovery mapping');
    }
    if (mappingByKey.has(mapping.discoveryKey)) duplicateAnonymousKeys.push(mapping.discoveryKey);
    mappingByKey.set(mapping.discoveryKey, mapping);
    if (mapping.matchedEntryId !== null && !registryKeys.has(mapping.matchedEntryId)) {
      invalidAnonymousOwners.push(mapping.discoveryKey);
    }
    if (mapping.matchedEntryId === null && !mapping.decisionReason?.trim()) {
      throw new Error(`Anonymous mapping ${mapping.discoveryKey} needs a decision reason`);
    }
  }
  const anonymousRows = rows.filter(row => !row.ids.length).map(row => {
    const mapping = mappingByKey.get(row.discoveryKey);
    return { key: row.key, discoveryKey: row.discoveryKey, sourceLine: row.sourceLine,
      location: row.location, control: row.control, handlerArea: row.handlerArea,
      matchedEntryId: mapping?.matchedEntryId ?? null, note: mapping?.note ?? null,
      decisionReason: mapping?.decisionReason ?? null };
  });
  const missingAnonymousRows = anonymousRows.filter(row => !row.discoveryKey || !mappingByKey.has(row.discoveryKey));
  const residualAnonymousRows = anonymousRows.filter(row => row.discoveryKey &&
    mappingByKey.get(row.discoveryKey)?.matchedEntryId === null);
  const discoveryAnonymousKeys = new Set(anonymousRows.map(row => row.discoveryKey).filter(Boolean));
  const extraAnonymousMappings = [...mappingByKey.keys()].filter(key => !discoveryAnonymousKeys.has(key)).sort();
  const duplicateDiscoveryAnonymousKeys = [...new Set(anonymousRows.map(row => row.discoveryKey)
    .filter((key, index, keys) => key && keys.indexOf(key) !== index))].sort();
  const indexedRows = [];
  const excludedRows = [];
  const unclassifiedRows = [];
  const conflictingRows = [];
  for (const row of idRows) {
    const classes = row.ids.map(id => ({ id, registryKeys: indexed.get(id) || [],
      exclusion: excluded.get(id) || null }));
    if (classes.some(item => item.registryKeys.length && item.exclusion)) conflictingRows.push(row.key);
    if (classes.some(item => !item.registryKeys.length && !item.exclusion)) {
      unclassifiedRows.push({ key: row.key, ids: row.ids, control: row.control });
    } else if (classes.every(item => item.registryKeys.length)) indexedRows.push(row.key);
    else if (classes.every(item => item.exclusion)) excludedRows.push(row.key);
    else conflictingRows.push(row.key);
  }
  const baselineIds = new Set(idRows.flatMap(row => row.ids));
  return { discoveryRows: rows.length, idBackedRows: idRows.length, anonymousRows,
    matchedAnonymousRows: anonymousRows.length - missingAnonymousRows.length - residualAnonymousRows.length,
    missingAnonymousRows, residualAnonymousRows, extraAnonymousMappings,
    duplicateAnonymousKeys: [...new Set(duplicateAnonymousKeys)].sort(),
    duplicateDiscoveryAnonymousKeys, invalidAnonymousOwners: [...new Set(invalidAnonymousOwners)].sort(),
    registryEntries: registryRows.length, explicitExclusions: exclusions.length,
    indexedRows: indexedRows.length, excludedRows: excludedRows.length,
    unclassifiedRows, conflictingRows: [...new Set(conflictingRows)].sort(),
    duplicateRegistryKeys: [...new Set(duplicateRegistryKeys)].sort(),
    duplicateExclusionIds: [...new Set(duplicateExclusionIds)].sort(),
    staleExclusionOwners: [...new Set(staleExclusionOwners)].sort(),
    // Controls added since the Phase 0 snapshot are expected here.
    registryIdsOutsideDiscovery: [...indexed.keys()].filter(id => !baselineIds.has(id)).sort(),
    exclusionIdsOutsideDiscovery: [...excluded.keys()].filter(id => !baselineIds.has(id)).sort() };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const rows = parseDiscoveryInventory(readFileSync(discoveryPath, 'utf8'));
  const { registryRows, exclusions, anonymousMappings } = await loadSettingsClassification();
  const comparison = auditDiscovery(rows, registryRows, exclusions, anonymousMappings);
  const unresolved = comparison.missingAnonymousRows.length + comparison.residualAnonymousRows.length +
    comparison.extraAnonymousMappings.length + comparison.duplicateAnonymousKeys.length +
    comparison.duplicateDiscoveryAnonymousKeys.length + comparison.invalidAnonymousOwners.length +
    comparison.unclassifiedRows.length +
    comparison.conflictingRows.length + comparison.duplicateRegistryKeys.length +
    comparison.duplicateExclusionIds.length + comparison.staleExclusionOwners.length;
  const report = { status: unresolved ? 'incomplete' : 'complete', discovery: discoveryPath,
    registryAdapter: resolve(root, 'src/settings/discovery-adapter.ts'),
    exclusions: resolve(root, 'src/settings/registry-exclusions.ts'), comparison };
  if (outputArg) writeFileSync(resolve(outputArg), JSON.stringify(report, null, 2) + '\n');
  console.log(`Discovery: ${comparison.discoveryRows} rows (${comparison.idBackedRows} with IDs, ${comparison.anonymousRows.length} anonymous)`);
  console.log(`Settings: ${comparison.registryEntries} entries, ${comparison.explicitExclusions} explicit exclusions`);
  console.log(`ID-backed rows: ${comparison.indexedRows} indexed, ${comparison.excludedRows} excluded, ${comparison.unclassifiedRows.length} unclassified, ${comparison.conflictingRows.length} conflicting`);
  console.log(`Anonymous rows: ${comparison.matchedAnonymousRows}/${comparison.anonymousRows.length} mapped; ${comparison.missingAnonymousRows.length} missing, ${comparison.residualAnonymousRows.length} unresolved, ${comparison.extraAnonymousMappings.length} stale mappings, ${comparison.invalidAnonymousOwners.length} bad owners`);
  console.log(`Integrity: ${comparison.duplicateRegistryKeys.length} duplicate registry keys, ${comparison.duplicateExclusionIds.length} duplicate exclusions, ${comparison.duplicateAnonymousKeys.length} duplicate anonymous mappings, ${comparison.duplicateDiscoveryAnonymousKeys.length} duplicate discovery keys, ${comparison.staleExclusionOwners.length} stale exclusion owners`);
  console.log(`Controls added since discovery: ${comparison.registryIdsOutsideDiscovery.length} indexed IDs, ${comparison.exclusionIdsOutsideDiscovery.length} excluded IDs`);
  if (strict && unresolved) process.exitCode = 1;
}
