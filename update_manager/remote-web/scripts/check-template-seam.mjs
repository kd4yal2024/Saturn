#!/usr/bin/env node
// Static gate for the template/bundle seam.
//
// The template consumes the bundle exclusively through the flat api object
// assigned to `globalThis.SaturnRemoteNext`. This check keeps that seam exact
// in both directions:
//   - every name the template references must exist in the api object, and
//   - every api entry must be referenced by the template.
// A failure in the first direction is a broken page; a failure in the second
// is dead surface creeping back in. Either fails the build.

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const remoteWebRoot = resolve(scriptDir, '..');
const templatePath = resolve(remoteWebRoot, '../templates/saturn-remote-next.html');
const entryPath = resolve(remoteWebRoot, 'src/remote-next-entry.ts');

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

function stripComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');
}

// Entries are `name` or `apiKey: localBinding`; the api key is the left side.
// Rejects spread/computed entries because this check cannot see through them.
function parseObjectEntryNames(blockBody, context) {
  const names = [];
  for (const rawEntry of stripComments(blockBody).split(',')) {
    const entry = rawEntry.trim();
    if (entry === '') {
      continue;
    }
    const name = entry.split(':')[0].trim();
    if (!IDENTIFIER.test(name)) {
      throw new Error(`${context}: cannot statically parse entry ${JSON.stringify(entry)}`);
    }
    names.push(name);
  }
  return names;
}

function templateUsedNames(template) {
  const used = new Set();

  // Direct property accesses: `_next.X` plus any `<qualifier>.SaturnRemoteNext.X`.
  for (const match of template.matchAll(/\b_next\.([A-Za-z_$][A-Za-z0-9_$]*)/g)) {
    used.add(match[1]);
  }
  for (const match of template.matchAll(/\bSaturnRemoteNext\.([A-Za-z_$][A-Za-z0-9_$]*)/g)) {
    used.add(match[1]);
  }

  // The destructuring block(s): `const { ... } = _next;`.
  const destructures = [...template.matchAll(/const\s*\{([\s\S]*?)\}\s*=\s*_next\s*;/g)];
  if (destructures.length === 0) {
    throw new Error('template: no `const { ... } = _next;` destructuring block found');
  }
  for (const match of destructures) {
    for (const name of parseObjectEntryNames(match[1], 'template destructuring block')) {
      used.add(name);
    }
  }
  return used;
}

function bundleApiNames(entrySource) {
  const start = entrySource.indexOf('const api = {');
  if (start === -1) {
    throw new Error('entry: `const api = {` not found');
  }
  const bodyStart = entrySource.indexOf('{', start) + 1;
  let depth = 1;
  let index = bodyStart;
  while (index < entrySource.length && depth > 0) {
    const char = entrySource[index];
    if (char === '{') depth += 1;
    if (char === '}') depth -= 1;
    index += 1;
  }
  if (depth !== 0) {
    throw new Error('entry: unbalanced braces in the api object literal');
  }
  const body = entrySource.slice(bodyStart, index - 1);
  if (/\{/.test(stripComments(body))) {
    throw new Error('entry: nested object in the api literal — flatten it or extend this parser');
  }
  const names = parseObjectEntryNames(body, 'entry api object');
  const seen = new Set();
  for (const name of names) {
    if (seen.has(name)) {
      throw new Error(`entry: duplicate api entry ${JSON.stringify(name)}`);
    }
    seen.add(name);
  }
  return seen;
}

const used = templateUsedNames(readFileSync(templatePath, 'utf8'));
const exported = bundleApiNames(readFileSync(entryPath, 'utf8'));

const missingFromApi = [...used].filter((name) => !exported.has(name)).sort();
const unusedByTemplate = [...exported].filter((name) => !used.has(name)).sort();

if (missingFromApi.length > 0) {
  console.error('Template references names the bundle api does not export:');
  for (const name of missingFromApi) console.error(`  - ${name}`);
}
if (unusedByTemplate.length > 0) {
  console.error('Bundle api exports names the template never references (dead surface):');
  for (const name of unusedByTemplate) console.error(`  - ${name}`);
}
if (missingFromApi.length > 0 || unusedByTemplate.length > 0) {
  console.error(
    `\ncheck-template-seam: FAILED (${used.size} template-used, ${exported.size} api entries)`,
  );
  process.exit(1);
}

console.log(
  `check-template-seam: OK — ${exported.size} api entries, all referenced by the template`,
);
