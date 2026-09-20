import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// @ts-expect-error - plain ESM helper, intentionally untyped
import { findUnresolvedIdentifiers, inlineScripts, lineOf } from './support/template-scope.mjs';

const TEMPLATE_DIR = resolve(process.cwd(), '../templates');

// Browser globals the templates legitimately rely on but Node does not define, plus
// globals published by the vendored <script src=...> bundles loaded ahead of them.
const AMBIENT_GLOBALS = [
  ...Object.getOwnPropertyNames(globalThis),
  'window',
  'document',
  'navigator',
  'location',
  'history',
  'localStorage',
  'sessionStorage',
  'alert',
  'confirm',
  'prompt',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'matchMedia',
  'WebSocket',
  'XMLHttpRequest',
  'ResizeObserver',
  'MutationObserver',
  'IntersectionObserver',
  'Element',
  'HTMLElement',
  'HTMLMediaElement',
  'AudioContext',
  'webkitAudioContext',
  'AudioWorkletNode',
  'MediaRecorder',
  'Notification',
  // vendored bundles
  'AnsiUp',
  'Chart',
  'SaturnShell',
];

const templates = readdirSync(TEMPLATE_DIR).filter((name) => name.endsWith('.html'));

describe('template inline scripts', () => {
  it('finds templates to check', () => {
    expect(templates.length).toBeGreaterThan(0);
  });

  it('detects a const referenced from a sibling scope', () => {
    const source = 'if (true) { const onlyHere = 1; } void onlyHere;';
    expect(findUnresolvedIdentifiers(source, { globals: AMBIENT_GLOBALS }))
      .toEqual([{ name: 'onlyHere', start: source.lastIndexOf('onlyHere') }]);
  });

  it.each(templates)('%s parses and every identifier resolves to a binding', (name) => {
    const html = readFileSync(resolve(TEMPLATE_DIR, name), 'utf8');
    const failures: string[] = [];

    for (const script of inlineScripts(html)) {
      let unresolved;
      try {
        unresolved = findUnresolvedIdentifiers(script.code, { globals: AMBIENT_GLOBALS });
      } catch (error) {
        failures.push(`syntax error near line ${lineOf(html, script.offset)}: ${(error as Error).message}`);
        continue;
      }
      for (const reference of unresolved) {
        failures.push(`${name}:${lineOf(html, script.offset + reference.start)} references undeclared '${reference.name}'`);
      }
    }

    expect(failures).toEqual([]);
  });
});
