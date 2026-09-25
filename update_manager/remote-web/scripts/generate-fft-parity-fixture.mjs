#!/usr/bin/env node
// Generate the Rust/browser FFT parity fixture.
//
// The bridge computes WAN spectrum rows with a Rust radix-2 FFT that claims to
// reproduce `src/dsp/fft.ts` exactly. This script runs the *real* fft.ts over a
// deterministic 2048-pair IQ block and writes both the input and the resulting
// dB bins, so the bridge test can assert the same numbers without shipping a
// second FFT implementation in the fixture generator.
//
// Usage: node scripts/generate-fft-parity-fixture.mjs
// Output: ../saturn-bridge/src/testdata/fft_parity_2048.json

import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..');
const outFile = resolve(root, '../saturn-bridge/src/testdata/fft_parity_2048.json');

const FFT_SIZE = 2048;
const SAMPLE_RATE_HZ = 384_000;

// Deterministic PRNG so regenerating the fixture is byte-identical.
function mulberry32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4_294_967_296;
  };
}

// Round to 6 decimals so the JSON numbers map to the same float32 in JS and Rust.
function round6(value) {
  return Number(value.toFixed(6));
}

function buildInput() {
  const random = mulberry32(0x5a7c0de);
  const iq = new Float32Array(FFT_SIZE * 2);
  for (let i = 0; i < FFT_SIZE; i += 1) {
    const n = i / FFT_SIZE;
    const iValue = 0.4 * Math.cos(2 * Math.PI * 37 * n)
      + 0.25 * Math.cos(2 * Math.PI * 411 * n + 0.3)
      + (random() - 0.5) * 0.05;
    const qValue = 0.3 * Math.sin(2 * Math.PI * 37 * n)
      + (random() - 0.5) * 0.05;
    iq[i * 2] = round6(iValue);
    iq[i * 2 + 1] = round6(qValue);
  }
  return iq;
}

const bundled = await build({
  entryPoints: [resolve(root, 'src/dsp/fft.ts')],
  bundle: true,
  write: false,
  format: 'esm',
  platform: 'node',
  target: 'node22',
});
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`;
const { FftProcessor } = await import(moduleUrl);

const iq = buildInput();
const bins = new FftProcessor(FFT_SIZE).transform(iq);

const fixture = {
  generator: 'update_manager/remote-web/scripts/generate-fft-parity-fixture.mjs',
  fftSize: FFT_SIZE,
  sampleRateHz: SAMPLE_RATE_HZ,
  dbOffset: -160,
  inputIqFloat32: Array.from(iq, (value) => Number(value)),
  expectedDb: Array.from(bins, (value) => Number(value.toFixed(6))),
};

mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, `${JSON.stringify(fixture)}\n`, 'utf8');
console.log(`wrote ${outFile} (${fixture.inputIqFloat32.length} floats in, ${fixture.expectedDb.length} bins out)`);
