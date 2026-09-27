import { readFileSync, readdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const bundle = readFileSync(resolve(root, 'dist/saturn-remote-next.js'), 'utf8');
const fontNames = [
  'IBMPlexSans-Regular-Latin1.woff2',
  'IBMPlexSans-Medium-Latin1.woff2',
  'IBMPlexMono-Regular-Latin1.woff2',
  'IBMPlexMono-Medium-Latin1.woff2',
];

let encodedBytes = 0;
for (const name of fontNames) {
  const bytes = readFileSync(resolve(root, 'assets/fonts', name));
  if (bytes.toString('ascii', 0, 4) !== 'wOF2') throw new Error(`${name} is not WOFF2`);
  const base64 = bytes.toString('base64');
  if (!bundle.includes(base64)) throw new Error(`${name} is missing from the IIFE bundle`);
  encodedBytes += Buffer.byteLength(base64) + Buffer.byteLength('data:font/woff2;base64,');
}

const sprite = readFileSync(resolve(root, 'assets/icons/sprite.svg'), 'utf8');
if (!sprite.includes('id="saturn-ui-icon-sprite"') || !bundle.includes('saturn-ui-icon-sprite')) {
  throw new Error('Icon sprite is missing from the IIFE bundle');
}
encodedBytes += Buffer.byteLength(JSON.stringify(sprite));

for (const licensePath of ['assets/fonts/OFL.txt', 'assets/icons/LICENSE.txt']) {
  const license = readFileSync(resolve(root, licensePath), 'utf8');
  if (!license || !bundle.includes(license.split(/\r?\n/, 1)[0])) {
    throw new Error(`${licensePath} is missing from the IIFE bundle`);
  }
  encodedBytes += Buffer.byteLength(JSON.stringify(license));
}

const extraAssets = readdirSync(resolve(root, 'dist')).filter((name) =>
  /\.(?:css|woff2?|svg)$/.test(name),
);
if (extraAssets.length) throw new Error(`Unexpected separate UI asset URLs: ${extraAssets.join(', ')}`);

const limit = 250_000;
if (encodedBytes >= limit) throw new Error(`Font + icon encoded payload ${encodedBytes} >= ${limit} bytes`);
console.log(`Font + icon encoded payload: ${encodedBytes} / ${limit} bytes, in one IIFE`);
