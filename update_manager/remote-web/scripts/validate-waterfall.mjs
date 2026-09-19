#!/usr/bin/env node
// Exercise the shipped renderer and shader in a real browser, including its
// physical ring seam. Layout-only tests cannot validate texture row ordering.
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const template = readFileSync(resolve(root, '../templates/saturn-remote-next.html'), 'utf8');
const start = template.indexOf('    class WaterfallRenderer {');
const end = template.indexOf('    const spectrumRenderer =', start);
if (start < 0 || end < 0) throw new Error('Could not locate the shipped waterfall renderer');
const renderer = template.slice(start, end);
const output = mkdtempSync(join(tmpdir(), 'saturn-waterfall-'));
const page = join(output, 'test.html');

function validate() {
  const checks = [];
  function assert(ok, message) {
    if (!ok) throw new Error(message);
  }
  function canvas(width, height) {
    const host = document.createElement('div');
    host.style.cssText = `width:${width}px;height:${height}px`;
    const element = document.createElement('canvas');
    host.appendChild(element);
    document.body.appendChild(host);
    return element;
  }
  const waterfall = new WaterfallRenderer(canvas(2, 512));
  assert(waterfall.gl, 'WebGL2 is required for this validation');
  const gl = waterfall.gl;
  gl.disable(gl.DITHER);
  for (const [palette, boundaries] of [['classic', [0.28, 0.58, 0.82]], ['ember', [0.45, 0.78]]]) {
    for (const boundary of boundaries) {
      const before = waterfall.colorForDb(boundary - 1e-6, 0, 1, palette);
      const after = waterfall.colorForDb(boundary + 1e-6, 0, 1, palette);
      assert(before.every((value, i) => Math.abs(value - after[i]) <= 1), `Discontinuous ${palette} palette at ${boundary}`);
    }
  }
  checks.push('continuous Classic and Ember color ramps');
  waterfall.colorForDb = (db) => [db, 0, 0];
  const uploads = [];
  const upload = gl.texSubImage2D.bind(gl);
  gl.texSubImage2D = (...args) => {
    uploads.push({ height: args[5], bytes: args[8].byteLength });
    return upload(...args);
  };
  function pixels() {
    waterfall.render();
    assert(gl.getError() === gl.NO_ERROR, 'WebGL error during rendering');
    const bytes = new Uint8Array(waterfall.canvas.width * waterfall.canvas.height * 4);
    gl.readPixels(0, 0, waterfall.canvas.width, waterfall.canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
    return (row, x = 0) => bytes[((waterfall.canvas.height - 1 - row) * waterfall.canvas.width + x) * 4];
  }
  for (let i = 0; i < 514; i++) waterfall.pushLine(new Float32Array([i % 251, (i + 1) % 251]), 0, 255);
  let pixel = pixels();
  for (let row = 0; row < 512; row++) {
    assert(Math.abs(pixel(row) - ((513 - row) % 251)) <= 1, `Wrapped history row ${row} is incorrect`);
  }
  assert(uploads.length === 514 && uploads.every((u) => u.height === 1 && u.bytes === 8), 'Steady-state uploads must contain only one row');
  checks.push('512-row wrap, newest-first order, single-row uploads');

  waterfall.shiftPixels(1);
  pixel = pixels();
  for (let row = 0; row < 512; row++) {
    assert(pixel(row, 0) === 0 && Math.abs(pixel(row, 1) - ((513 - row) % 251)) <= 1, `Right tuning shift failed at row ${row}`);
  }
  waterfall.shiftPixels(-1);
  pixel = pixels();
  assert(pixel(0, 1) === 0 && Math.abs(pixel(0, 0) - (513 % 251)) <= 1, 'Left tuning shift failed');
  checks.push('tuning shifts preserve wrapped history');

  // At double vertical size the logical edges must not blend newest/oldest.
  waterfall.canvas.parentElement.style.height = '1024px';
  pixel = pixels();
  assert(Math.abs(pixel(0) - (513 % 251)) <= 1, 'Newest edge blended with oldest');
  assert(Math.abs(pixel(1023) - 2) <= 1, 'Oldest edge blended with newest');
  for (let row = 0; row < 1024; row++) {
    const logical = Math.max(0, Math.min(511, (row + 0.5) / 2 - 0.5));
    const low = Math.floor(logical);
    const high = Math.min(511, low + 1);
    const mix = logical - low;
    const expected = ((513 - low) % 251) * (1 - mix) + ((513 - high) % 251) * mix;
    assert(Math.abs(pixel(row) - expected) <= 1, `Scaled ring interpolation failed at row ${row}`);
  }
  checks.push('scaled history edges');
  waterfall.clear();
  pixel = pixels();
  assert(pixel(0) === 0 && pixel(1023) === 0, 'Clear retained history');
  waterfall.canvas.parentElement.style.height = '512px';
  waterfall.canvas.parentElement.style.width = '4px';
  waterfall.pushLine(new Float32Array([11, 22, 33, 44]), 0, 255);
  pixel = pixels();
  assert(pixel(0, 0) === 11 && pixel(0, 3) === 44 && pixel(1) === 0, 'Width change retained stale history');
  checks.push('clear and FFT-width change');

  const fallbackCanvas = canvas(2, 512);
  const getContext = fallbackCanvas.getContext.bind(fallbackCanvas);
  fallbackCanvas.getContext = (kind, ...args) => kind === 'webgl2' ? null : getContext(kind, ...args);
  const fallback = new WaterfallRenderer(fallbackCanvas);
  fallback.colorForDb = (db) => [db, 0, 0];
  fallback.pushLine(new Float32Array([99, 99]), 0, 255);
  fallback.render();
  fallback.render();
  const fallbackPixels = fallback.ctx.getImageData(0, 0, 2, 2).data;
  assert(fallbackPixels[0] === 99 && fallbackPixels[8] === 0, 'Canvas redraw duplicated a history row');
  fallback.pushLine(new Float32Array([55, 55]), 0, 255);
  fallback.clear();
  fallback.render();
  assert(fallback.ctx.getImageData(0, 0, 1, 1).data[0] === 0, 'Canvas clear replayed a pending row');
  checks.push('Canvas2D fallback redraw and clear');
  return checks;
}

writeFileSync(page, `<!doctype html><body><pre id="result"></pre><script>
const _next = { smoothWaterfallBins: (bins) => bins };
const normalizeWaterfallPalette = (palette) => palette;
const clampWaterfallContrast = (contrast) => contrast;
${renderer}
try {
  document.getElementById('result').textContent = JSON.stringify({pass:true,checks:(${validate.toString()})()});
} catch (error) {
  document.getElementById('result').textContent = JSON.stringify({pass:false,error:String(error.stack || error)});
}
</script>`);
const result = spawnSync(process.env.CHROMIUM || 'google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--no-first-run',
  '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--force-device-scale-factor=1',
  `--user-data-dir=${join(output, 'profile')}`, '--dump-dom', pathToFileURL(page).href,
], { encoding: 'utf8', timeout: 45000, maxBuffer: 4 * 1024 * 1024 });
const match = result.stdout?.match(/<pre id="result">([^<]+)<\/pre>/);
if (result.error || result.status !== 0 || !match) {
  throw new Error(`Browser validation failed: ${result.error || result.stderr}\nArtifacts: ${output}`);
}
const report = JSON.parse(match[1].replaceAll('&quot;', '"').replaceAll('&gt;', '>').replaceAll('&lt;', '<').replaceAll('&amp;', '&'));
writeFileSync(join(output, 'result.json'), JSON.stringify(report, null, 2));
if (!report.pass) throw new Error(`${report.error}\nArtifacts: ${output}`);
for (const check of report.checks) console.log(`PASS ${check}`);
console.log(`Output: ${output}`);
