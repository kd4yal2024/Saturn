#!/usr/bin/env node
// Browser geometry audit for the Phase 2 responsive console. This uses the
// shipped IIFE and the real static template, with radio runtime removed.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const templatePath = resolve(root, '../templates/saturn-remote-next.html');
const bundlePath = resolve(root, 'dist/saturn-remote-next.js');
const baselinePath = resolve(root, 'scripts/fixtures/ui-redesign-control-ids.json');
const output = process.argv.find((arg) => arg.startsWith('--output='))?.slice(9)
  || mkdtempSync(join(tmpdir(), 'saturn-ui-tiers-'));
const widthsArg = process.argv.find((arg) => arg.startsWith('--widths='))?.slice(9);
const widths = widthsArg ? widthsArg.split(',').map(Number) : [360, 390, 600, 768, 960, 1280, 1440, 1920, 2560];
if (widths.some((width) => !Number.isInteger(width) || width < 320)) throw new Error('Invalid --widths list');
const heightFor = (width) => width < 600 ? (width <= 360 ? 780 : 844)
  : width < 960 ? 900 : width < 1440 ? 900 : width < 1920 ? 960 : width < 2560 ? 1080 : 1440;
const scenarios = widths.flatMap((width) => ['dark', 'light'].map((theme) => ({
  name: `${width}-${theme}`, width, height: heightFor(width), theme, layout: 'desktop',
}))).concat(
  { name: '390-legacy-phone', width: 390, height: 844, theme: 'dark', layout: 'phone' },
  { name: '1280-legacy-phone', width: 1280, height: 900, theme: 'dark', layout: 'phone' },
);

for (const [path, label] of [[templatePath, 'template'], [bundlePath, 'built IIFE'], [baselinePath, 'baseline ID fixture']]) {
  if (!existsSync(path)) throw new Error(`${label} missing: ${path}`);
}
mkdirSync(output, { recursive: true });
const source = readFileSync(templatePath, 'utf8');
const bundle = readFileSync(bundlePath, 'utf8');
const baselineIds = JSON.parse(readFileSync(baselinePath, 'utf8'));
if (!Array.isArray(baselineIds) || baselineIds.length < 400) throw new Error('Invalid baseline ID fixture');
const marker = source.indexOf('<!-- Runtime provided by SaturnRemoteNext bundle -->');
if (marker < 0) throw new Error('Template runtime marker missing');
const pttHandlerStart = source.indexOf('    function bindPttButton() {');
const pttHandlerEnd = source.indexOf('    async function goLive() {', pttHandlerStart);
if (pttHandlerStart < 0 || pttHandlerEnd < 0) throw new Error('Template PTT handler markers missing');
const pttHandlerSource = source.slice(pttHandlerStart, pttHandlerEnd);
const setupHandlerStart = source.indexOf('    function syncSetupPanels() {');
const setupHandlerEnd = source.indexOf('    async function loadRemoteProfilesFromServer(', setupHandlerStart);
if (setupHandlerStart < 0 || setupHandlerEnd < 0) throw new Error('Template Settings handler markers missing');
const setupHandlerSource = source.slice(setupHandlerStart, setupHandlerEnd);
const presentationStart = source.indexOf('          (function wireTuneAndMeterPresentation() {');
const presentationEnd = source.indexOf('        </script>', presentationStart);
if (presentationStart < 0 || presentationEnd < 0) throw new Error('Template tuning presentation script missing');
const presentationSource = source.slice(presentationStart, presentationEnd).trim();
const interactionStart = source.indexOf('      document.querySelectorAll("#mode-grid .mode-btn").forEach',
  source.indexOf('    function initInteractions() {'));
const interactionEnd = source.indexOf('      $("vfo-step-down-btn")', interactionStart);
if (interactionStart < 0 || interactionEnd < 0) throw new Error('Template mode/band handlers missing');
const modeBandInteractionSource = source.slice(interactionStart, interactionEnd);
const phonePanelStart = source.indexOf('    function loadPhonePanelState() {');
const phonePanelEnd = source.indexOf('    function revealShellPanel(', phonePanelStart);
if (phonePanelStart < 0 || phonePanelEnd < 0) throw new Error('Template phone disclosure handlers missing');
const phonePanelSource = source.slice(phonePanelStart, phonePanelEnd);
const controlContextStart = source.indexOf('    function applyControlContext(value, persist = true) {');
const controlContextEnd = source.indexOf('    function initControlContextRail() {', controlContextStart);
if (controlContextStart < 0 || controlContextEnd < 0) throw new Error('Template context handler markers missing');
const controlContextSource = source.slice(controlContextStart, controlContextEnd);

function pageFor(scenario) {
  // Remove radio runtime to prevent socket/audio requests. The fixture below
  // performs its actual context-rail DOM move before measuring layout.
  const staticMarkup = source.slice(0, marker).replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '');
  const withState = staticMarkup.replace('<html lang="en">',
    `<html lang="en" data-theme="${scenario.theme}" data-layout="${scenario.layout}">`);
  if (withState === staticMarkup) throw new Error('Template root element changed');
  const iife = bundle.replaceAll('</script', '<\\/script');
  return withState.replace('</head>', `<script>${iife}</script>\n</head>`) + '\n</body></html>\n';
}

function fullRuntimePageFor(scenario) {
  const withState = source.replace('<html lang="en">',
    `<html lang="en" data-theme="${scenario.theme}" data-layout="${scenario.layout}">`);
  const bundleTag = /<script\s+src="\/remote-assets\/remote-next\.js[^"]*"[\s\S]*?<\/script>/;
  if (!bundleTag.test(withState)) throw new Error('Served bundle script tag missing');
  const withBundle = withState.replace(bundleTag,
    `<script>${bundle.replaceAll('</script', '<\\/script')}\nwindow.saturnRemoteBundleLoaded=true;</script>`);
  // The real app persists frequency lock; give this isolated runtime probe a
  // known starting state before its boot script reads browser storage.
  return withBundle.replace(marker,
    `<script>localStorage.setItem('saturn.remote.freqLock', '0');</script>\n${marker}`);
}

for (const scenario of scenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, pageFor(scenario));
}
const pttScenario = { name: 'ptt-390', width: 390, height: 844, theme: 'dark', layout: 'desktop' };
pttScenario.file = join(output, 'ptt-390.html');
writeFileSync(pttScenario.file, pageFor(pttScenario));
const settingsScenarios = [390, 1280].map(width => ({
  name: `settings-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of settingsScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, pageFor(scenario));
}
const presentationScenarios = [390, 1440].map(width => ({
  name: `presentation-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of presentationScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, pageFor(scenario));
}
const settingsIndexScenarios = [390, 1280].map(width => ({
  name: `settings-index-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of settingsIndexScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const exactEntryScenarios = [390, 1280].map(width => ({
  name: `rx-exact-entry-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of exactEntryScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const systemRouteScenarios = [390, 1280].map(width => ({
  name: `system-route-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of systemRouteScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const mainScreenScenarios = [390, 1280].map(width => ({
  name: `main-screen-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of mainScreenScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const restoreScenarios = [390, 1280].map(width => ({
  name: `restore-defaults-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of restoreScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const pendingEntryScenarios = [390, 1280].map(width => ({
  name: `pending-entries-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of pendingEntryScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const registryRouteScenarios = [390, 1280].map(width => ({
  name: `registry-routes-${width}`, width, height: heightFor(width), theme: 'dark', layout: 'desktop',
}));
for (const scenario of registryRouteScenarios) {
  scenario.file = join(output, scenario.name + '.html');
  writeFileSync(scenario.file, fullRuntimePageFor(scenario));
}
const disclosureScenarios = [
  { name: 'disclosure-390', width: 390, height: 844, theme: 'dark', layout: 'desktop' },
  { name: 'disclosure-390-legacy', width: 390, height: 844, theme: 'dark', layout: 'phone' },
  { name: 'disclosure-1280-legacy', width: 1280, height: 900, theme: 'dark', layout: 'phone' },
];
for (const scenario of disclosureScenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, pageFor(scenario));
}
const vfoScenario = { name: 'vfo-runtime-390', width: 390, height: 844, theme: 'dark', layout: 'desktop' };
vfoScenario.file = join(output, `${vfoScenario.name}.html`);
writeFileSync(vfoScenario.file, fullRuntimePageFor(vfoScenario));

const chrome = spawn(process.env.CHROMIUM || 'google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--no-first-run',
  '--disable-background-networking', '--disable-extensions', '--disable-sync',
  '--hide-scrollbars', '--mute-audio', '--allow-file-access-from-files',
  '--remote-debugging-pipe', '--force-device-scale-factor=1',
  `--user-data-dir=${join(output, 'profile')}`, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
let stderr = '';
chrome.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
let nextId = 0;
const pending = new Map();
let buffer = '';
chrome.on('error', (error) => {
  for (const waiter of pending.values()) waiter.reject(error);
  pending.clear();
});
chrome.stdio[4].on('data', (chunk) => {
  buffer += chunk.toString();
  let end;
  while ((end = buffer.indexOf('\0')) >= 0) {
    const raw = buffer.slice(0, end);
    buffer = buffer.slice(end + 1);
    if (!raw) continue;
    const message = JSON.parse(raw);
    const waiter = pending.get(message.id);
    if (!waiter) continue;
    pending.delete(message.id);
    message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
  }
});
function command(method, params = {}, sessionId) {
  return new Promise((resolveCall, rejectCall) => {
    const id = ++nextId;
    pending.set(id, { resolve: resolveCall, reject: rejectCall });
    chrome.stdio[3].write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + '\0');
  });
}
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
const watchdog = setTimeout(() => { console.error('Chromium timed out:', stderr.slice(-2000)); chrome.kill(); process.exit(2); }, 180_000);

const prepareFixtureExpression = (scenario) => `(() => {
  const state = { layoutMode: ${JSON.stringify(scenario.layout)} };
  const $ = id => document.getElementById(id);
  const normalizeRadioControlContext = globalThis.SaturnRemoteNextBundle?.SaturnRemoteNext?.normalizeRadioControlContext;
  if (typeof normalizeRadioControlContext !== 'function') throw Error('Context normalizer missing from bundle');
  ${controlContextSource}
  const consoleLayout = document.querySelector('.console-layout');
  const audioStrip = document.querySelector('[data-phone-panel="audio"]');
  const rightRail = document.querySelector('.right-rail');
  if (!consoleLayout || !audioStrip || !rightRail) throw Error('Context rail source nodes missing');
  let rail = document.getElementById('radio-context-rail');
  if (!rail) {
    rail = document.createElement('aside');
    rail.id = 'radio-context-rail';
    rail.className = 'context-rail';
    rail.setAttribute('aria-label', 'Radio controls');
    const tabs = document.createElement('div');
    tabs.className = 'context-tabs';
    tabs.setAttribute('role', 'tablist');
    tabs.setAttribute('aria-label', 'Radio control context');
    for (const context of ['rx', 'dsp', 'tx']) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'context-tab';
      button.dataset.controlContext = context;
      button.setAttribute('role', 'tab');
      button.textContent = context.toUpperCase();
      button.addEventListener('click', () => window.__uiFixtureApplyControlContext(context));
      tabs.appendChild(button);
    }
    rail.appendChild(tabs);
    consoleLayout.insertAdjacentElement('afterend', rail);
  }
  rail.appendChild(audioStrip);
  rail.appendChild(rightRail);
  window.__uiFixtureApplyControlContext = context => applyControlContext(context, false);
  window.__uiFixtureApplyControlContext('rx');
  return { railParent: rail.parentElement?.className, audioParent: audioStrip.parentElement?.id,
    txParent: rightRail.parentElement?.id };
})()`;

// Bind the production pointer handler with a harmless TX stub. The static
// fixture intentionally omits the rest of the radio runtime and its sockets.
const installPttFixtureExpression = `(() => {
  const state = { pointerPttActive: false, pointerPttRequestId: 0, pointerPttPointerId: null,
    keyboardPttActive: false, moxRequested: false, txEnabled: false, micCapturing: false, connected: true };
  const calls = [];
  const events = [];
  const $ = id => document.getElementById(id);
  const holdPttActive = () => state.keyboardPttActive || state.pointerPttActive;
  const clearPointerPttTracking = () => {
    state.pointerPttActive = false;
    state.pointerPttPointerId = null;
    state.pointerPttRequestId += 1;
  };
  const updateUi = () => {};
  const logEvent = () => {};
  const recordOperatorFault = () => {};
  const lockTx = () => {};
  const armTxReady = () => {};
  const setPtt = async (active, options = {}) => {
    calls.push({ active, options });
    state.moxRequested = active;
    if (!active && state.pointerPttActive) clearPointerPttTracking();
    return true;
  };
  ${pttHandlerSource}
  const button = $('ptt-btn');
  if (!button || document.querySelectorAll('#ptt-btn').length !== 1) throw Error('Single PTT button missing');
  button.disabled = false;
  for (const type of ['pointerdown', 'pointerup', 'pointercancel', 'lostpointercapture']) {
    button.addEventListener(type, event => events.push({ type, pointerId: event.pointerId }), true);
  }
  bindPttButton();
  window.__pttFixture = { state, calls, events, button };
  return true;
})()`;

// Use the template's Settings search/navigation functions, while skipping the
// radio runtime and its network-dependent field synchronization.
const installSettingsFixtureExpression = `(() => {
  const state = { setupPanel: 'profiles', setupDspPanel: 'nr' };
  const $ = id => document.getElementById(id);
  const _next = globalThis.SaturnRemoteNextBundle?.SaturnRemoteNext;
  const normalizeSetupPanelId = _next?.normalizeSetupPanelId;
  if (typeof normalizeSetupPanelId !== 'function') throw Error('Setup panel normalizer missing from bundle');
  const UI_SETTINGS_LAST_SECTION_KEY = 'saturn.ui.settings.lastSection';
  const UI_THEME_KEY = 'saturn.ui.theme';
  const UI_PHONE_RX_OPEN_KEY = 'saturn.ui.phone.rxOpen';
  const UI_PHONE_TX_OPEN_KEY = 'saturn.ui.phone.txOpen';
  const SETUP_PANEL_PREF_KEY = 'saturn.remote.setupPanel';
  const isResponsivePhoneConsole = () => document.documentElement.dataset.layout !== 'phone' &&
    document.querySelector('.app').clientWidth < 600;
  const syncSetupMenuFields = () => syncSetupPanels();
  ${setupHandlerSource}
  const errors = [];
  window.addEventListener('error', event => errors.push(event.message));
  window.addEventListener('unhandledrejection', event => errors.push(String(event.reason)));
  $('setup-menu-btn').addEventListener('click', () => setSetupMenuOpen(true));
  $('header-setup-btn').addEventListener('click', () => setSetupMenuOpen(true));
  $('phone-menu-settings-btn').addEventListener('click', () => setSetupMenuOpen(true));
  $('setup-close-btn').addEventListener('click', () => setSetupMenuOpen(false));
  $('settings-search').addEventListener('input', updateSetupSearch);
  window.__settingsFixture = { state, errors };
  return true;
})()`;

// The presentation IIFE is taken verbatim from the template. The existing
// radio mode/band click handlers are also bound verbatim, with radio effects
// replaced by local state and a command log so no bridge is contacted.
const installPresentationFixtureExpression = `(() => {
  const state = { mode: 'USB', frequency: 14200000, bandMemory: {} };
  const commands = [];
  const errors = [];
  window.addEventListener('error', event => errors.push(event.message));
  window.addEventListener('unhandledrejection', event => errors.push(String(event.reason)));
  document.querySelector('#band-grid .band-btn[data-band="14200000"]')?.classList.add('active');
  ${presentationSource}
  const selectReceiveMode = mode => { state.mode = mode; return true; };
  const saveRadioPrefs = () => {};
  const sendTci = command => commands.push(command);
  const sendRxFilterBand = () => {};
  const sendTxFilterBand = () => {};
  const bandKeyForFrequency = hz => String(hz);
  const rememberCurrentBandSettings = () => {};
  const setFrequency = hz => { state.frequency = hz; };
  const applyBandMemoryForFrequency = () => false;
  const sendCurrentRadioPrefsToBridge = () => {};
  const resumeRxMediaAfterBandChange = () => {};
  const persistLocalSettingsFallback = () => {};
  const currentRemoteSettings = () => ({});
  const scheduleRemoteSettingsSave = () => {};
  const updateUi = () => {
    document.querySelectorAll('#mode-grid .mode-btn').forEach(button =>
      button.classList.toggle('active', button.dataset.mode === state.mode));
    document.querySelectorAll('#band-grid .band-btn').forEach(button =>
      button.classList.toggle('active', Number(button.dataset.band) === state.frequency));
  };
  ${modeBandInteractionSource}
  window.__presentationFixture = { state, commands, errors };
  return true;
})()`;

const installDisclosureFixtureExpression = (scenario) => `(() => {
  const state = { layoutMode: ${JSON.stringify(scenario.layout)}, phonePanels: {} };
  const $ = id => document.getElementById(id);
  const PHONE_PANEL_DEFAULTS = Object.freeze({ session: false, audio: false, routing: true,
    tuning: false, demod: false, display: false, log: true, telemetry: true, tx: false });
  const UI_PHONE_RX_OPEN_KEY = 'saturn.ui.phone.rxOpen';
  const UI_PHONE_TX_OPEN_KEY = 'saturn.ui.phone.txOpen';
  const sanitizePhonePanels = globalThis.SaturnRemoteNextBundle?.SaturnRemoteNext?.sanitizePhonePanels;
  if (typeof sanitizePhonePanels !== 'function') throw Error('Phone panel sanitizer missing from bundle');
  const spectrumRenderer = { resize() {} };
  const waterfallRenderer = { resize() {} };
  const scheduleRemoteSettingsSave = () => {};
  ${phonePanelSource}
  initPhonePanels();
  window.__disclosureFixture = { state, togglePhonePanel, syncPhonePanels };
  return true;
})()`;

const measureExpression = (scenario) => `(() => {
  const scenario = ${JSON.stringify(scenario)};
  const baselineIds = ${JSON.stringify(baselineIds)};
  const round = x => Math.round(x * 10) / 10;
  const rect = e => { if (!e) return null; const r = e.getBoundingClientRect();
    return { x: round(r.x), y: round(r.y), width: round(r.width), height: round(r.height), right: round(r.right), bottom: round(r.bottom) }; };
  const visible = e => {
    if (!e || e.closest('[hidden], [aria-hidden="true"]')) return false;
    const closed = e.closest('details:not([open])');
    if (closed && !closed.querySelector('summary')?.contains(e)) return false;
    return e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden' &&
      getComputedStyle(e).opacity !== '0' && e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().height > 0;
  };
  const firstVisible = selectors => selectors.map(s => document.querySelector(s)).find(visible) || null;
  const selector = e => e ? (e.id ? '#' + e.id : e.className && typeof e.className === 'string'
    ? '.' + e.className.trim().split(/\\s+/).join('.') : e.tagName.toLowerCase()) : null;
  const hitTest = e => {
    if (!visible(e)) return { visible: false, onTop: false, hit: null };
    const r = e.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return { visible: true, onTop: !!hit && (hit === e || e.contains(hit)), hit: selector(hit) };
  };
  const insideX = e => { const r = e.getBoundingClientRect(); return r.right > 0 && r.left < scenario.width && r.left >= -1 && r.right <= scenario.width + 1; };
  const failures = [];
  const check = (ok, code, detail) => { if (!ok) failures.push({ code, detail }); };
  const warnings = [];
  const warn = (condition, code, detail) => { if (condition) warnings.push({ code, detail }); };
  const tier = scenario.width < 600 ? 'phone' : scenario.width < 960 ? 'tablet'
    : scenario.width < 1440 ? 'desktop' : 'wide';
  const app = document.querySelector('.app');
  const bar = firstVisible(['.app-bar', '#app-bar', '.console-header']);
  const left = firstVisible(['.left-rail', '[data-app-region="left"]']);
  const center = firstVisible(['.stage.console-stage', '.center-stage', '.center-rail', '[data-app-region="center"]', '.display-panel']);
  const right = firstVisible(['.right-rail', '[data-app-region="right"]']);
  const vfo = firstVisible(['#dds-readout', '[data-ui="vfo-readout"]']);
  const txBadge = firstVisible(['#app-tx-state-badge', '#tx-status-badge', '#tx-state-badge', '.tx-badge', '[data-ui="tx-badge"]', '#operator-rxtx-pill', '#tx-zone-state']);
  const display = firstVisible(['#display-well', '.display-stack', '#spectrum-shell', '[data-ui="display-well"]']);
  const spectrum = firstVisible(['#terrain-canvas', '#spectrum-shell']);
  const waterfall = firstVisible(['#waterfall-shell']);
  const dock = document.getElementById('mobile-control-dock');
  const txHost = document.querySelector('#tx-safety-host.phone-tx-bar');
  const txArm = document.getElementById('tx-arm-btn');
  const ptt = document.getElementById('ptt-btn');
  const stickyTx = firstVisible(['#tx-safety-host.phone-tx-bar', '#tx-sticky-bar', '#tx-safety-bar', '#mobile-tx-bar', '.tx-sticky-bar', '.tx-safety-bar', '.mobile-tx-bar', '[data-tx-sticky]']);
  const audioStrip = document.querySelector('.audio-control-strip');
  const rail = document.getElementById('radio-context-rail');
  const ids = [...document.querySelectorAll('[id]')].map(e => e.id);
  const duplicateIds = [...new Set(ids.filter((id, index) => ids.indexOf(id) !== index))];
  const duplicateTxIds = duplicateIds.filter(id => /(?:^|-)tx|^ptt|^mox/.test(id));
  const missingIds = baselineIds.filter(id => !document.getElementById(id));
  const pageWidth = Math.max(document.documentElement.scrollWidth, document.body.scrollWidth);
  const hasSafeAreaRule = element => !!element && [...document.querySelectorAll('style')].some(style => {
    const css = style.textContent || '';
    return css.includes('safe-area-inset-bottom') &&
      ((element.id && css.includes('#' + element.id)) || [...element.classList].some(name => css.includes('.' + name)));
  });
  const overlayAboveHost = id => {
    const overlay = document.getElementById(id);
    if (!overlay || !visible(txHost)) return { above: false, hit: null };
    const wasHidden = overlay.hidden;
    overlay.hidden = false;
    const r = txHost.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    const result = { above: !!hit && (overlay === hit || overlay.contains(hit)), hit: selector(hit) };
    overlay.hidden = wasHidden;
    return result;
  };
  const pttSpanWithArmHidden = () => {
    const zone = txHost?.querySelector('#tx-zone');
    const row = ptt?.closest('.tx-action-row');
    if (!zone || !row || !txArm) return { spans: false, reason: 'TX geometry missing' };
    const wasHidden = txArm.hidden;
    txArm.hidden = true;
    const style = getComputedStyle(zone);
    const available = zone.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    const actual = row.getBoundingClientRect().width;
    const pttWidth = ptt.getBoundingClientRect().width;
    const pttHit = hitTest(ptt);
    txArm.hidden = wasHidden;
    return { spans: actual >= available - 6 && pttWidth >= actual - 2 && pttHit.onTop,
      available: round(available), rowWidth: round(actual), pttWidth: round(pttWidth), pttHit };
  };
  check(!!document.getElementById('saturn-ui-foundation'), 'bundle-css-missing', null);
  check(missingIds.length === 0, 'old-control-ids-missing', missingIds);
  check(duplicateIds.length === 0, 'duplicate-dom-ids', duplicateIds);
  check(duplicateTxIds.length === 0, 'duplicate-tx-ids', duplicateTxIds);
  let exclusiveContexts = null;
  let armedPttSpan = null;
  let overlayHits = null;
  if (scenario.layout !== 'phone') {
    check(!!rail && audioStrip?.parentElement === rail && document.querySelector('.right-rail')?.parentElement === rail,
      'context-rail-reparent-missing', { audioParent: audioStrip?.parentElement?.id, txParent: document.querySelector('.right-rail')?.parentElement?.id });
    const txPanel = document.querySelector('.right-rail [data-phone-panel="tx"]');
    const rxControl = audioStrip.querySelector('[data-control-context="rx"]');
    const dspControl = audioStrip.querySelector('[data-control-context="dsp"]');
    const status = audioStrip.querySelector('.audio-control-status');
    const rxState = { receive: visible(rxControl), dsp: visible(dspControl), transmit: visible(txPanel),
      safety: visible(txHost), audioStatus: visible(status) };
    window.__uiFixtureApplyControlContext('dsp');
    const dspState = { receive: visible(rxControl), dsp: visible(dspControl), transmit: visible(txPanel),
      safety: visible(txHost), audioStatus: visible(status) };
    window.__uiFixtureApplyControlContext('tx');
    const txState = { receive: visible(rxControl), dsp: visible(dspControl), transmit: visible(txPanel),
      safety: visible(txHost), audioStatus: visible(status), selected: rail?.dataset.activeContext };
    window.__uiFixtureApplyControlContext('rx');
    exclusiveContexts = { rxState, dspState, txState };
    check(rxState.receive && !rxState.dsp && !rxState.transmit && rxState.safety && rxState.audioStatus &&
      !dspState.receive && dspState.dsp && !dspState.transmit && dspState.safety && !dspState.audioStatus &&
      !txState.receive && !txState.dsp && txState.transmit && txState.safety && txState.selected === 'tx',
      'radio-context-details-not-exclusive', exclusiveContexts);
    check(innerWidth <= scenario.width + 1, 'viewport-auto-scaled', { requestedWidth: scenario.width, layoutViewportWidth: innerWidth, visualScale: visualViewport?.scale });
    check(pageWidth <= scenario.width + 1, 'horizontal-scroll', { pageWidth, requestedWidth: scenario.width });
    check(visible(vfo) && insideX(vfo), 'vfo-not-visible', { selector: selector(vfo), rect: rect(vfo) });
    check(visible(txBadge) && insideX(txBadge), 'tx-badge-not-visible', { selector: selector(txBadge), rect: rect(txBadge) });
    check(!!vfo && vfo.getBoundingClientRect().top >= -1 && vfo.getBoundingClientRect().top < scenario.height,
      'vfo-below-initial-viewport', rect(vfo));
    check(!!txBadge && txBadge.getBoundingClientRect().top >= -1 && txBadge.getBoundingClientRect().top < scenario.height,
      'tx-badge-below-initial-viewport', rect(txBadge));
    check(visible(display) && insideX(display), 'display-not-visible', { selector: selector(display), rect: rect(display) });
    const maxDisplayTop = tier === 'phone' ? 700 : tier === 'tablet' ? 500 : 400;
    warn(!!display && display.getBoundingClientRect().top >= maxDisplayTop,
      'display-below-initial-viewport', { maxTop: maxDisplayTop, rect: rect(display) });
    check(!!bar && insideX(bar), 'app-bar-clipped', rect(bar));
  }
  if (scenario.layout === 'phone') {
    check(innerWidth <= scenario.width + 1 && pageWidth <= scenario.width + 1,
      'legacy-phone-horizontal-scroll', { requestedWidth: scenario.width, layoutWidth: innerWidth, pageWidth });
    check(visible(dock) && insideX(dock), 'legacy-phone-dock-unreachable', rect(dock));
    check(visible(txHost) && insideX(txHost), 'legacy-phone-tx-host-unreachable', rect(txHost));
    if (visible(dock) && visible(txHost)) {
      const dockRect = dock.getBoundingClientRect();
      const hostRect = txHost.getBoundingClientRect();
      check(getComputedStyle(txHost).position === 'fixed' && hostRect.bottom <= dockRect.top + 2,
        'legacy-phone-tx-host-not-above-dock', { host: rect(txHost), dock: rect(dock) });
      check(hitTest(txHost).onTop, 'legacy-phone-tx-host-occluded',
        { host: rect(txHost), hit: hitTest(txHost).hit });
      check(hitTest(txArm).onTop && hitTest(ptt).onTop, 'legacy-phone-tx-controls-occluded',
        { arm: hitTest(txArm), ptt: hitTest(ptt) });
      if (scenario.width === 390) {
        armedPttSpan = pttSpanWithArmHidden();
        check(armedPttSpan.spans, 'legacy-phone-armed-ptt-not-full-width', armedPttSpan);
      }
      overlayHits = { menu: overlayAboveHost('phone-menu-sheet'), detail: overlayAboveHost('operator-detail-overlay') };
      check(overlayHits.menu.above && overlayHits.detail.above,
        'legacy-phone-sheet-under-tx-host', overlayHits);
      check(hasSafeAreaRule(txHost), 'legacy-phone-tx-host-safe-area-rule-missing', selector(txHost));
    }
    check(visible(vfo) && visible(display), 'legacy-phone-primary-regions-missing', { vfo: rect(vfo), display: rect(display) });
  } else {
    check(!visible(dock), 'responsive-legacy-dock-visible', rect(dock));
    check(!!app, 'app-container-missing', null);
    const expectedBar = tier === 'phone' ? 44 : 48;
    check(!!bar && Math.abs(bar.getBoundingClientRect().height - expectedBar) <= 2,
      'app-bar-height', { expected: expectedBar, actual: rect(bar)?.height });
    if (tier === 'desktop' || tier === 'wide') {
      check(!!left && Math.abs(left.getBoundingClientRect().width - 240) <= 8,
        'left-rail-width', { expected: 240, actual: rect(left)?.width });
      const expectedRight = tier === 'wide' ? 340 : 300;
      check(!!right && Math.abs(right.getBoundingClientRect().width - expectedRight) <= 8,
        'right-rail-width', { expected: expectedRight, actual: rect(right)?.width });
      if (center) check(center.getBoundingClientRect().width >= 200, 'center-too-narrow', rect(center));
      check(!!display && display.getBoundingClientRect().height >= 360,
        'display-well-too-short', { minimum: 360, actual: rect(display)?.height });
    } else if (tier === 'tablet') {
      check(!!spectrum && spectrum.getBoundingClientRect().height >= 240,
        'tablet-spectrum-too-short', { minimum: 240, actual: rect(spectrum)?.height });
      check(!!waterfall && waterfall.getBoundingClientRect().height >= 120,
        'tablet-waterfall-too-short', { minimum: 120, actual: rect(waterfall)?.height });
    } else {
      check(!!spectrum && spectrum.getBoundingClientRect().height >= 180,
        'phone-spectrum-too-short', { minimum: 180, actual: rect(spectrum)?.height });
      check(!!waterfall && waterfall.getBoundingClientRect().height >= 80,
        'phone-waterfall-too-short', { minimum: 80, actual: rect(waterfall)?.height });
      const stickyStyle = stickyTx ? getComputedStyle(stickyTx) : null;
      check(!!stickyTx, 'phone-tx-sticky-bar-missing', null);
      if (stickyTx) {
        check(['fixed', 'sticky'].includes(stickyStyle.position) && stickyTx.getBoundingClientRect().bottom <= innerHeight + 1,
          'phone-tx-bar-not-sticky', { position: stickyStyle.position, rect: rect(stickyTx) });
        check(hitTest(stickyTx).onTop, 'phone-tx-bar-occluded',
          { host: rect(stickyTx), hit: hitTest(stickyTx).hit });
        check(hitTest(txArm).onTop && hitTest(ptt).onTop, 'phone-tx-controls-occluded',
          { arm: hitTest(txArm), ptt: hitTest(ptt) });
        if (scenario.width <= 390) {
          armedPttSpan = pttSpanWithArmHidden();
          check(armedPttSpan.spans, 'phone-armed-ptt-not-full-width', armedPttSpan);
        }
        overlayHits = { menu: overlayAboveHost('phone-menu-sheet'), detail: overlayAboveHost('operator-detail-overlay') };
        check(overlayHits.menu.above && overlayHits.detail.above,
          'phone-sheet-under-tx-bar', overlayHits);
        check(hasSafeAreaRule(stickyTx), 'phone-tx-bar-safe-area-rule-missing', selector(stickyTx));
      }
    }
  }
  const horizontallyReachable = e => {
    for (let ancestor = e.parentElement; ancestor; ancestor = ancestor.parentElement) {
      const overflow = getComputedStyle(ancestor).overflowX;
      if (['auto', 'scroll'].includes(overflow) && ancestor.scrollWidth > ancestor.clientWidth + 1) return true;
    }
    return false;
  };
  const visibleControls = [...document.querySelectorAll('button[id], input[id], select[id], textarea[id], a[id][href], [role="button"][id]')]
    .filter(visible);
  const offscreenControls = visibleControls.filter(e => !insideX(e) && !horizontallyReachable(e))
    .map(e => ({ id: e.id, rect: rect(e) }));
  if (scenario.layout !== 'phone') {
    check(offscreenControls.length === 0, 'visible-controls-offscreen', offscreenControls.slice(0, 20));
  }
  return {
    scenario: scenario.name, tier, theme: scenario.theme, layout: scenario.layout,
    viewport: { requestedWidth: scenario.width, requestedHeight: scenario.height,
      layoutWidth: innerWidth, layoutHeight: innerHeight, visualScale: visualViewport?.scale }, pageWidth,
    ids: { expected: baselineIds.length, missing: missingIds },
    duplicateIds, exclusiveContexts, armedPttSpan, overlayHits,
    controls: { visible: visibleControls.length, offscreen: offscreenControls },
    hitTests: { txHost: hitTest(txHost), txArm: hitTest(txArm), ptt: hitTest(ptt), dock: hitTest(dock) },
    regions: Object.fromEntries(Object.entries({ app, bar, left, center, right, vfo, txBadge, display, spectrum, waterfall, dock, txHost, stickyTx })
      .map(([name, e]) => [name, { selector: selector(e), rect: rect(e) }])),
    failures, warnings, ok: failures.length === 0,
  };
})()`;

const reports = [];
const pttReports = [];
const settingsReports = [];
const presentationReports = [];
const disclosureReports = [];
const vfoReports = [];
const settingsIndexReports = [];
const exactEntryReports = [];
const systemRouteReports = [];
const mainScreenReports = [];
const restoreReports = [];
const pendingEntryReports = [];
const registryRouteReports = [];
try {
  const { targetId } = await command('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await command('Target.attachToTarget', { targetId, flatten: true });
  const call = (method, params = {}) => command(method, params, sessionId);
  const evaluate = async (expression) => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  await call('Page.enable');
  await call('Runtime.enable');
  for (const scenario of scenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1,
      mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      await pause(50);
      ready = await evaluate('document.readyState === "complete" && !!document.getElementById("saturn-ui-foundation")').catch(() => false);
      if (ready) break;
    }
    if (!ready) throw new Error(`${scenario.name}: page or bundled CSS did not load`);
    await evaluate('document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))');
    await evaluate(prepareFixtureExpression(scenario));
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const report = await evaluate(measureExpression(scenario));
    report.screenshot = `${scenario.name}.png`;
    reports.push(report);
    const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}.png`), Buffer.from(png.data, 'base64'));
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name} ${scenario.width}x${scenario.height}: ${report.failures.map(f => f.code).join(', ') || 'all checks'}${report.warnings.length ? ` (${report.warnings.length} warning${report.warnings.length === 1 ? '' : 's'})` : ''}`);
    writeFileSync(join(output, 'summary.json'), JSON.stringify({ generatedAt: new Date().toISOString(), reports }, null, 2));
  }
  for (const releaseType of ['touchEnd', 'touchCancel']) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: 390, height: 844, deviceScaleFactor: 1, mobile: true,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(pttScenario.file).href });
    for (let attempt = 0; attempt < 100; attempt++) {
      await pause(50);
      if (await evaluate('document.readyState === "complete" && !!document.getElementById("saturn-ui-foundation")').catch(() => false)) break;
      if (attempt === 99) throw new Error('PTT fixture did not load');
    }
    await evaluate(prepareFixtureExpression(pttScenario));
    await evaluate(installPttFixtureExpression);
    const point = await evaluate(`(() => { const r = document.getElementById('ptt-btn').getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    const snapshot = () => evaluate(`(() => { const f = window.__pttFixture; const button = document.getElementById('ptt-btn');
      return { active: f.state.pointerPttActive, pointerId: f.state.pointerPttPointerId,
        captured: f.state.pointerPttPointerId !== null && button.hasPointerCapture(f.state.pointerPttPointerId),
        sameButton: button === f.button, count: document.querySelectorAll('#ptt-btn').length,
        calls: f.calls.slice(), events: f.events.slice() }; })()`);
    await call('Input.dispatchTouchEvent', {
      type: 'touchStart', touchPoints: [{ x: point.x, y: point.y, id: 7, radiusX: 1, radiusY: 1 }],
    });
    await pause(50);
    const down = await snapshot();
    await call('Emulation.setDeviceMetricsOverride', {
      width: 960, height: 900, deviceScaleFactor: 1, mobile: true,
    });
    await pause(50);
    const resized = await snapshot();
    let dispatchedRelease = false;
    if (resized.active) {
      await call('Input.dispatchTouchEvent', { type: releaseType, touchPoints: [] });
      dispatchedRelease = true;
      await pause(50);
    }
    const released = await snapshot();
    const failures = [];
    if (!down.active || !down.captured || down.calls.filter(call => call.active).length !== 1)
      failures.push('pointerdown-did-not-capture-and-key');
    if (!resized.sameButton || resized.count !== 1)
      failures.push('ptt-button-replaced-or-duplicated-on-resize');
    if (resized.active && !resized.captured)
      failures.push('pointer-capture-lost-without-release');
    if (released.active || released.captured || released.calls.filter(call => !call.active).length !== 1)
      failures.push('pointer-release-did-not-unkey');
    if (dispatchedRelease && !released.events.some(event => event.type === (releaseType === 'touchEnd' ? 'pointerup' : 'pointercancel')))
      failures.push('requested-pointer-release-event-missing');
    const report = { releaseType, from: 390, to: 960, point, down, resized,
      dispatchedRelease, released, failures, ok: failures.length === 0 };
    pttReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} PTT 390→960 ${releaseType}: ${failures.join(', ') || 'captured and released'}`);
  }
  for (const scenario of settingsScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    for (let attempt = 0; attempt < 100; attempt++) {
      await pause(50);
      if (await evaluate('document.readyState === "complete" && !!document.getElementById("saturn-ui-foundation")').catch(() => false)) break;
      if (attempt === 99) throw new Error(`${scenario.name}: Settings fixture did not load`);
    }
    await evaluate(prepareFixtureExpression(scenario));
    await evaluate(installSettingsFixtureExpression);
    const opened = await evaluate(`(() => {
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
      if (!trigger) {
        trigger = document.getElementById('phone-menu-btn');
        if (visible(trigger)) {
          trigger.focus(); trigger.click();
          document.getElementById('phone-menu-settings-btn').click();
        }
      } else {
        trigger.focus(); trigger.click();
      }
      if (!trigger) throw Error('Visible Settings trigger missing');
      window.__settingsFixture.trigger = trigger;
      return { trigger: trigger.id, menuOpen: !document.getElementById('setup-menu').hidden,
        expanded: document.getElementById('header-setup-btn').getAttribute('aria-expanded') }; })()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const searched = await evaluate(`(() => {
      const input = document.getElementById('settings-search');
      input.value = 'spectrum floor';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      const buttons = [...document.querySelectorAll('#settings-search-results .settings-search-result')];
      const match = buttons.find(button => button.querySelector('.settings-search-result-label')?.textContent?.trim().toLowerCase() === 'spectrum floor');
      return { count: buttons.length, match: !!match, resultsVisible: !document.getElementById('settings-search-results').hidden,
        bodyHidden: document.querySelector('#setup-menu .setup-menu-body').hidden }; })()`);
    const selected = await evaluate(`(() => {
      const match = [...document.querySelectorAll('#settings-search-results .settings-search-result')]
        .find(button => button.querySelector('.settings-search-result-label')?.textContent?.trim().toLowerCase() === 'spectrum floor');
      if (match) match.click();
      return { clicked: !!match }; })()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    Object.assign(selected, await evaluate(`(() => ({ panel: window.__settingsFixture.state.setupPanel,
      panelVisible: !document.getElementById('setup-panel-display').hidden,
      tabSelected: document.getElementById('setup-tab-display').getAttribute('aria-selected'),
      focus: document.activeElement?.id, resultsHidden: document.getElementById('settings-search-results').hidden,
      searchValue: document.getElementById('settings-search').value }))()`));
    const closeHit = await evaluate(`(() => {
      const button = document.getElementById('setup-close-btn');
      const r = button.getBoundingClientRect();
      const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return { label: button.textContent.trim(), onTop: top === button || button.contains(top),
        hitId: top?.id || null, width: r.width, height: r.height };
    })()`);
    const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}.png`), Buffer.from(png.data, 'base64'));
    const closed = await evaluate(`(() => {
      document.getElementById('setup-close-btn').click();
      return { menuClosed: document.getElementById('setup-menu').hidden,
        trigger: window.__settingsFixture.trigger.id, focus: document.activeElement?.id,
        errors: window.__settingsFixture.errors.slice() }; })()`);
    const failures = [];
    if (!opened.menuOpen || opened.expanded !== 'true') failures.push('settings-did-not-open');
    if (!searched.match || !searched.resultsVisible || !searched.bodyHidden) failures.push('display-setting-search-missing');
    if (!selected.clicked || selected.panel !== 'display' || !selected.panelVisible ||
      selected.tabSelected !== 'true' || selected.focus !== 'display-spectrum-floor' ||
      !selected.resultsHidden || selected.searchValue) failures.push('search-result-did-not-focus-display-control');
    if (closeHit.label !== 'Close' || !closeHit.onTop || closeHit.width < 36 || closeHit.height < 36)
      failures.push('settings-close-obscured-or-unlabeled');
    if (!closed.menuClosed || closed.focus !== opened.trigger) failures.push('settings-close-did-not-restore-trigger-focus');
    if (closed.errors.length) failures.push('settings-console-errors');
    const report = { scenario: scenario.name, opened, searched, selected, closeHit, closed, failures, ok: failures.length === 0 };
    settingsReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'search, target focus, and focus return'}`);
  }
  for (const scenario of presentationScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    for (let attempt = 0; attempt < 100; attempt++) {
      await pause(50);
      if (await evaluate('document.readyState === "complete" && !!document.getElementById("saturn-ui-foundation")').catch(() => false)) break;
      if (attempt === 99) throw new Error(`${scenario.name}: presentation fixture did not load`);
    }
    await evaluate(prepareFixtureExpression(scenario));
    await evaluate(installPresentationFixtureExpression);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const geometry = await evaluate(`(() => {
      const data = e => { if (!e) return null; const r = e.getBoundingClientRect();
        const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return { id: e.id, visible: e.getClientRects().length > 0, x: r.x, y: r.y,
          width: r.width, height: r.height, right: r.right,
          onTop: hit === e || e.contains(hit) }; };
      const live = document.getElementById('go-live-btn');
      const setup = ['setup-menu-btn','header-setup-btn'].map(id => document.getElementById(id))
        .find(e => e?.getClientRects().length);
      return { goLive: data(live), setup: data(setup), pageWidth: document.documentElement.scrollWidth }; })()`);
    let interactions = null;
    if (scenario.width === 390) {
      interactions = await evaluate(`(() => {
        const visible = e => !!e && e.getClientRects().length > 0;
        const modeGrid = document.getElementById('mode-grid');
        const bandGrid = document.getElementById('band-grid');
        const extraMode = modeGrid.querySelector('.mode-btn[data-mode="CWL"]');
        const extraBand = bandGrid.querySelector('.band-btn[data-band="17100000"]');
        const before = { modeHidden: !visible(extraMode), bandHidden: !visible(extraBand) };
        document.getElementById('mode-more-btn').click();
        document.getElementById('band-more-btn').click();
        const expanded = { modeVisible: visible(extraMode), bandVisible: visible(extraBand),
          sameModeNode: modeGrid.querySelector('.mode-btn[data-mode="CWL"]') === extraMode,
          sameBandNode: bandGrid.querySelector('.band-btn[data-band="17100000"]') === extraBand };
        modeGrid.querySelector('.mode-btn[data-mode="USB"]').focus();
        modeGrid.querySelector('.mode-btn[data-mode="USB"]').dispatchEvent(new KeyboardEvent('keydown',
          { key: 'ArrowRight', bubbles: true, cancelable: true }));
        const mode = { state: window.__presentationFixture.state.mode,
          active: modeGrid.querySelector('.mode-btn.active')?.dataset.mode,
          focused: document.activeElement?.dataset.mode,
          command: window.__presentationFixture.commands.at(-1) };
        const visibleBands = [...bandGrid.querySelectorAll('.band-btn')].filter(visible);
        const selectedBandIndex = visibleBands.findIndex(button => button.dataset.band === '14200000');
        const expectedNextBand = visibleBands[(selectedBandIndex + 1) % visibleBands.length]?.dataset.band;
        bandGrid.querySelector('.band-btn[data-band="14200000"]').focus();
        bandGrid.querySelector('.band-btn[data-band="14200000"]').dispatchEvent(new KeyboardEvent('keydown',
          { key: 'ArrowRight', bubbles: true, cancelable: true }));
        const band = { frequency: window.__presentationFixture.state.frequency, expectedNextBand,
          active: bandGrid.querySelector('.band-btn.active')?.dataset.band,
          focused: document.activeElement?.dataset.band };
        document.getElementById('vfo-mode-tag').click();
        const modePillFocus = document.activeElement?.dataset.mode;
        document.getElementById('vfo-band-tag').click();
        const bandPillFocus = document.activeElement?.dataset.band;
        const meter = document.getElementById('instrument-meter-mode');
        const analog = { selectable: !!meter.querySelector('option[value="analog"]'),
          detailsVisible: visible(document.getElementById('meter-analog-option')),
          svgVisible: visible(document.getElementById('smeter-svg')) };
        return { before, expanded, mode, band, modePillFocus, bandPillFocus, analog,
          errors: window.__presentationFixture.errors.slice() }; })()`);
      await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
      interactions.aria = await evaluate(`(() => ({
        mode: document.querySelector('#mode-grid .mode-btn.active')?.getAttribute('aria-checked'),
        band: document.querySelector('#band-grid .band-btn.active')?.getAttribute('aria-checked') }))()`);
    }
    const failures = [];
    if (scenario.width === 390) {
      if (!interactions.before.modeHidden || !interactions.before.bandHidden ||
        !interactions.expanded.modeVisible || !interactions.expanded.bandVisible ||
        !interactions.expanded.sameModeNode || !interactions.expanded.sameBandNode)
        failures.push('more-controls-did-not-reveal-original-buttons');
      if (interactions.mode.state !== 'LSB' || interactions.mode.active !== 'LSB' ||
        interactions.mode.focused !== 'LSB' || interactions.mode.command !== 'modulation:0,LSB;' || interactions.aria.mode !== 'true')
        failures.push('mode-arrow-did-not-run-original-handler');
      if (!interactions.band.expectedNextBand ||
        interactions.band.frequency !== Number(interactions.band.expectedNextBand) ||
        interactions.band.active !== interactions.band.expectedNextBand ||
        interactions.band.focused !== interactions.band.expectedNextBand || interactions.aria.band !== 'true')
        failures.push('band-arrow-did-not-run-original-handler');
      if (interactions.modePillFocus !== 'LSB' || interactions.bandPillFocus !== interactions.band.expectedNextBand)
        failures.push('vfo-pill-did-not-focus-active-choice');
      if (interactions.analog.selectable || interactions.analog.detailsVisible || interactions.analog.svgVisible)
        failures.push('analog-meter-still-visible');
      if (interactions.errors.length) failures.push('presentation-console-errors');
    } else {
      for (const control of [geometry.goLive, geometry.setup]) {
        if (!control?.visible || !control.onTop || control.x < 0 || control.right > scenario.width + 1 ||
          control.width < 36 || control.height < 36) failures.push(`${control?.id || 'missing'}-desktop-target-unreachable`);
      }
      if (geometry.pageWidth > scenario.width + 1) failures.push('desktop-horizontal-scroll');
    }
    const report = { scenario: scenario.name, geometry, interactions, failures, ok: failures.length === 0 };
    presentationReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'tuning/meter/target checks'}`);
  }
  for (const scenario of disclosureScenarios) {
    const setViewport = () => call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    const load = async () => {
      await setViewport();
      await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
      await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
      for (let attempt = 0; attempt < 100; attempt++) {
        await pause(50);
        if (await evaluate('document.readyState === "complete" && !!document.getElementById("saturn-ui-foundation")').catch(() => false)) break;
        if (attempt === 99) throw new Error(`${scenario.name}: disclosure fixture did not load`);
      }
      await evaluate(prepareFixtureExpression(scenario));
      await evaluate(installDisclosureFixtureExpression(scenario));
      await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    };
    await setViewport();
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    for (let attempt = 0; attempt < 100; attempt++) {
      await pause(50);
      if (await evaluate('document.readyState === "complete"').catch(() => false)) break;
      if (attempt === 99) throw new Error(`${scenario.name}: local storage fixture did not load`);
    }
    await evaluate(`(() => {
      localStorage.setItem('saturn.remote.phonePanels', JSON.stringify({ audio: true, tx: false }));
      ${scenario.layout === 'phone' ? "localStorage.setItem('saturn.ui.phone.rxOpen', 'true'); localStorage.setItem('saturn.ui.phone.txOpen', 'true');" : "localStorage.removeItem('saturn.ui.phone.rxOpen'); localStorage.removeItem('saturn.ui.phone.txOpen');"}
    })()`);
    await load();
    const snapshot = () => evaluate(`(() => {
      const audio = document.querySelector('[data-phone-panel="audio"]');
      const tx = document.querySelector('[data-phone-panel="tx"]');
      const hit = e => { const r = e.getBoundingClientRect(); const top = document.elementFromPoint(r.x + r.width/2, r.y + r.height/2);
        return top === e || e.contains(top); };
      const arm = document.getElementById('tx-arm-btn');
      const ptt = document.getElementById('ptt-btn');
      return { audioCollapsed: audio.dataset.phoneCollapsed, txCollapsed: tx.dataset.phoneCollapsed,
        audioBodyVisible: audio.querySelector('.panel-body').getClientRects().length > 0,
        txBodyVisible: tx.querySelector('.panel-body').getClientRects().length > 0,
        audioToggleVisible: audio.querySelector('.panel-toggle-btn').getClientRects().length > 0,
        txToggleVisible: tx.querySelector('.panel-toggle-btn').getClientRects().length > 0,
        armOnTop: hit(arm), pttOnTop: hit(ptt),
        responsiveRxKey: localStorage.getItem('saturn.ui.phone.rxOpen'),
        responsiveTxKey: localStorage.getItem('saturn.ui.phone.txOpen'),
        legacyPanels: JSON.parse(localStorage.getItem('saturn.remote.phonePanels') || '{}') }; })()`);
    const initial = await snapshot();
    const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}.png`), Buffer.from(png.data, 'base64'));
    await evaluate(`window.__uiFixtureApplyControlContext('tx')`);
    const txInitial = await snapshot();
    await evaluate(`document.querySelector('[data-phone-panel="tx"] .panel-toggle-btn').click()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const toggled = await snapshot();
    await load();
    await evaluate(`window.__uiFixtureApplyControlContext('tx')`);
    const reloaded = await snapshot();
    const failures = [];
    if (scenario.layout !== 'phone') {
      if (initial.audioCollapsed !== 'false' || !initial.audioBodyVisible ||
        initial.txCollapsed !== 'true' || initial.txBodyVisible ||
        !initial.audioToggleVisible || initial.txToggleVisible || !initial.armOnTop || !initial.pttOnTop ||
        txInitial.txToggleVisible !== true || txInitial.txBodyVisible !== false)
        failures.push('responsive-phone-disclosure-default-or-sticky-tx');
      if (toggled.txCollapsed !== 'false' || !toggled.txBodyVisible || toggled.responsiveTxKey !== 'true' ||
        toggled.legacyPanels.tx !== false) failures.push('responsive-tx-disclosure-not-independent');
      if (reloaded.txCollapsed !== 'false' || !reloaded.txBodyVisible || !reloaded.armOnTop || !reloaded.pttOnTop)
        failures.push('responsive-tx-disclosure-not-persisted');
    } else {
      if (initial.audioCollapsed !== 'true' || initial.audioBodyVisible ||
        initial.txCollapsed !== 'false' || initial.txBodyVisible ||
        !initial.armOnTop || !initial.pttOnTop || !txInitial.txToggleVisible || !txInitial.txBodyVisible)
        failures.push('legacy-phone-state-affected-by-responsive-keys');
      if (toggled.txCollapsed !== 'true' || toggled.txBodyVisible || toggled.legacyPanels.tx !== true ||
        toggled.responsiveTxKey !== 'true') failures.push('legacy-phone-toggle-not-independent');
      if (reloaded.txCollapsed !== 'true' || reloaded.txBodyVisible ||
        !reloaded.armOnTop || !reloaded.pttOnTop) failures.push('legacy-phone-state-not-persisted');
    }
    const report = { scenario: scenario.name, initial, txInitial, toggled, reloaded, failures, ok: failures.length === 0 };
    disclosureReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'panel visibility, sticky TX, and independent persistence'}`);
  }
  await call('Emulation.setDeviceMetricsOverride', {
    width: vfoScenario.width, height: vfoScenario.height, deviceScaleFactor: 1, mobile: true,
  });
  await call('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 });
  await call('Page.navigate', { url: pathToFileURL(vfoScenario.file).href });
  let vfoReady = false;
  for (let attempt = 0; attempt < 160; attempt++) {
    await pause(50);
    vfoReady = await evaluate(`document.readyState === 'complete' &&
      document.querySelectorAll('#dds-readout .freq-digit').length > 0`).catch(() => false);
    if (vfoReady) break;
  }
  if (!vfoReady) throw new Error('Full VFO runtime did not initialize');
  const vfoResult = await evaluate(`(() => {
    const readout = document.getElementById('dds-readout');
    const overlay = document.getElementById('freq-entry-overlay');
    const lock = document.getElementById('freq-lock-btn');
    const cancel = document.getElementById('freq-entry-cancel-btn');
    const functionType = typeof window.openFrequencyEntry;
    const initialLocked = lock.classList.contains('active');
    if (initialLocked) lock.click();
    const unlocked = !lock.classList.contains('active');
    readout.focus();
    readout.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    const enterOpened = !overlay.hidden;
    cancel.click();
    const enterClosed = overlay.hidden;
    readout.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true }));
    const doubleClickOpened = !overlay.hidden;
    cancel.click();
    lock.click();
    const locked = lock.classList.contains('active');
    readout.focus();
    readout.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    const lockedBlocked = overlay.hidden;
    return { functionType, initialLocked, unlocked, enterOpened, enterClosed, doubleClickOpened, locked, lockedBlocked,
      runtimeError: window.saturnDiagLastError || null }; })()`);
  const vfoFailures = [];
  if (vfoResult.functionType !== 'function') vfoFailures.push('frequency-entry-handler-not-exposed');
  if (!vfoResult.unlocked) vfoFailures.push('vfo-could-not-unlock-for-direct-entry');
  if (!vfoResult.enterOpened || !vfoResult.enterClosed) vfoFailures.push('vfo-enter-did-not-open-and-close-entry');
  if (!vfoResult.doubleClickOpened) vfoFailures.push('vfo-double-click-did-not-open-entry');
  if (!vfoResult.locked || !vfoResult.lockedBlocked) vfoFailures.push('vfo-lock-did-not-block-entry');
  const vfoReport = { scenario: vfoScenario.name, ready: vfoReady,
    result: vfoResult, failures: vfoFailures, ok: vfoFailures.length === 0 };
  vfoReports.push(vfoReport);
  console.log(`${vfoReport.ok ? 'PASS' : 'FAIL'} ${vfoScenario.name}: ${vfoFailures.join(', ') || 'Enter, double-click, and lock gate'}`);
  for (const scenario of settingsIndexScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let ready = false;
    for (let attempt = 0; attempt < 160; attempt++) {
      await pause(50);
      ready = await evaluate(`document.readyState === 'complete' &&
        document.querySelectorAll('#settings-section-tabs .settings-section-tab').length === 11`).catch(() => false);
      if (ready) break;
    }
    if (!ready) throw new Error(`${scenario.name}: full Settings runtime did not initialize`);
    await evaluate(`(() => { localStorage.setItem('saturn.remote.layout', 'desktop');
      if (typeof window.applyLayout !== 'function') throw Error('Layout handler missing');
      window.applyLayout('desktop', false, false); return true; })()`);
    const opened = await evaluate(`(() => {
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
      if (!trigger) {
        trigger = document.getElementById('phone-menu-btn');
        if (!visible(trigger)) throw Error('Visible Settings trigger missing');
        trigger.focus(); trigger.click();
        const route = document.getElementById('phone-menu-settings-btn');
        if (!visible(route)) throw Error('Phone Settings route missing');
        route.click();
      } else { trigger.focus(); trigger.click(); }
      window.__settingsIndexTargets = { trigger, floor: document.getElementById('display-spectrum-floor'),
        volume: document.getElementById('rx-volume'), pause: document.getElementById('display-pause') };
      const tabs = [...document.querySelectorAll('#settings-section-tabs .settings-section-tab')];
      return { trigger: trigger.id, menuOpen: !document.getElementById('setup-menu').hidden,
        layout: document.documentElement.dataset.layout,
        sectionCount: tabs.length, sections: tabs.map(tab => tab.dataset.settingsSection),
        indexActive: document.querySelector('.setup-stack').dataset.indexActive,
        pageVisible: visible(document.getElementById('settings-section-page')),
        selected: tabs.find(tab => tab.getAttribute('aria-selected') === 'true')?.dataset.settingsSection,
        sourceCounts: { floor: document.querySelectorAll('#display-spectrum-floor').length,
          volume: document.querySelectorAll('#rx-volume').length } }; })()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const navigation = await evaluate(`(() => {
      const tab = name => document.querySelector('.settings-section-tab[data-settings-section="' + name + '"]');
      const selected = () => document.querySelector('.settings-section-tab[aria-selected="true"]')?.dataset.settingsSection;
      tab('receive').click();
      const clicked = { selected: selected(), label: document.getElementById('settings-section-page').getAttribute('aria-label'),
        stored: localStorage.getItem('saturn.ui.settings.lastSection') };
      tab('receive').focus();
      tab('receive').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
      const right = { selected: selected(), focus: document.activeElement?.dataset.settingsSection };
      tab('transmit').dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }));
      const left = { selected: selected(), focus: document.activeElement?.dataset.settingsSection };
      tab('receive').dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true, cancelable: true }));
      const end = { selected: selected(), focus: document.activeElement?.dataset.settingsSection };
      tab('about').dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true, cancelable: true }));
      const home = { selected: selected(), focus: document.activeElement?.dataset.settingsSection };
      tab('display').click();
      return { clicked, right, left, end, home, displaySelected: selected(),
        displayRows: document.querySelectorAll('#settings-section-page .settings-entry-row').length }; })()`);
    const indexPng = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}.png`), Buffer.from(indexPng.data, 'base64'));
    const legacyClicked = await evaluate(`(() => {
      const row = [...document.querySelectorAll('#settings-section-page button.settings-entry-row')]
        .find(item => item.querySelector('.settings-entry-label')?.textContent?.trim() === 'Spectrum floor');
      row?.click();
      return { found: !!row }; })()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const legacy = { ...legacyClicked, ...await evaluate(`(() => {
      const target = document.getElementById('display-spectrum-floor');
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      return { indexActive: document.querySelector('.setup-stack').dataset.indexActive,
        panelVisible: visible(document.getElementById('setup-panel-display')),
        pageHidden: document.getElementById('settings-section-page').hidden,
        returnVisible: visible(document.getElementById('settings-index-return')),
        targetVisible: visible(target), focus: document.activeElement?.id,
        sameTarget: target === window.__settingsIndexTargets.floor,
        targetCount: document.querySelectorAll('#display-spectrum-floor').length }; })()`) };
    const back = await evaluate(`(() => {
      document.getElementById('settings-index-return').click();
      const selected = document.querySelector('.settings-section-tab[aria-selected="true"]');
      return { indexActive: document.querySelector('.setup-stack').dataset.indexActive,
        pageVisible: !document.getElementById('settings-section-page').hidden,
        selected: selected?.dataset.settingsSection, focus: document.activeElement?.dataset.settingsSection,
        returnHidden: document.getElementById('settings-index-return').hidden }; })()`);
    const indexInventory = await evaluate(`(() => {
      const inventory = { direct: [], navigation: [], static: [] };
      for (const tab of document.querySelectorAll('.settings-section-tab')) {
        tab.click();
        for (const row of document.querySelectorAll('#settings-section-page .settings-entry-row')) {
          const id = row.dataset.settingsEntry;
          if (row.querySelector('.settings-entry-control')) inventory.direct.push(id);
          else if (row.tagName === 'BUTTON') inventory.navigation.push(id);
          else inventory.static.push(id);
        }
      }
      document.querySelector('.settings-section-tab[data-settings-section="display"]').click();
      return inventory; })()`);
    const proxies = await evaluate(`(() => {
      const cases = [
        ['receive', 'receive.volume', 'rx-volume'],
        ['receive', 'receive.filterLow', 'filter-low'],
        ['receive', 'receive.filterHigh', 'filter-high'],
        ['radio', 'radio.antenna', 'rx-antenna'],
      ];
      return cases.map(([section, entryId, targetId]) => {
        document.querySelector('.settings-section-tab[data-settings-section="' + section + '"]').click();
        const row = document.querySelector('#settings-section-page [data-settings-entry="' + entryId + '"]');
        const proxy = row?.querySelector('.settings-entry-control');
        const target = document.getElementById(targetId);
        if (!proxy || !target) return { entryId, missing: true };
        const before = { target: target.value, proxy: proxy.value, disabled: target.disabled,
          proxyDisabled: proxy.disabled, count: document.querySelectorAll('#' + targetId).length,
          proxyCount: document.querySelectorAll('#' + proxy.id).length };
        const observed = [];
        const capture = event => observed.push({ type: event.type, value: target.value });
        target.addEventListener('input', capture, true);
        target.addEventListener('change', capture, true);
        // A disconnected radio may disable a safe RX control. Temporarily enable
        // the source in this isolated page so native forwarding can be exercised.
        target.disabled = false;
        target.dispatchEvent(new Event('change', { bubbles: true }));
        observed.length = 0;
        let requested;
        if (proxy.tagName === 'SELECT') {
          requested = [...proxy.options].find(option => !option.disabled && option.value !== target.value)?.value;
        } else {
          const step = Math.max(Number(proxy.step) || 1, 0.5);
          const value = Number(proxy.value);
          requested = String(value + step <= Number(proxy.max) ? value + step : value - step);
        }
        if (requested !== undefined) {
          proxy.value = requested;
          proxy.dispatchEvent(new Event('input', { bubbles: true }));
          proxy.dispatchEvent(new Event('change', { bubbles: true }));
        }
        const forward = { requested, observed: observed.slice(), target: target.value,
          proxy: proxy.value, synced: target.value === proxy.value };
        target.value = before.target;
        target.dispatchEvent(new Event('input', { bubbles: true }));
        target.dispatchEvent(new Event('change', { bubbles: true }));
        const reverse = { target: target.value, proxy: proxy.value, synced: target.value === proxy.value };
        target.disabled = true;
        target.dispatchEvent(new Event('change', { bubbles: true }));
        const disabledMirrored = proxy.disabled;
        target.disabled = before.disabled;
        target.dispatchEvent(new Event('change', { bubbles: true }));
        const disabledRestored = proxy.disabled === target.disabled;
        return { entryId, targetId, before, forward, reverse, disabledMirrored, disabledRestored,
          sameSource: target === document.getElementById(targetId) };
      });
    })()`);
    await evaluate(`document.querySelector('.settings-section-tab[data-settings-section="display"]').click()`);
    const outsideClicked = await evaluate(`(() => {
      const row = document.querySelector('#settings-section-page button.settings-entry-row[data-settings-entry="display.pause"]');
      const source = document.getElementById('display-pause');
      let sourceClicks = 0;
      source?.addEventListener('click', () => { sourceClicks++; }, { once: true });
      row?.click();
      return { found: !!row, sourceClicks,
        stored: localStorage.getItem('saturn.ui.settings.lastSection') }; })()`);
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    const outside = { ...outsideClicked, ...await evaluate(`(() => {
      const target = document.getElementById('display-pause');
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      return { menuClosed: document.getElementById('setup-menu').hidden, targetVisible: visible(target),
        focus: document.activeElement?.id, sameTarget: target === window.__settingsIndexTargets.pause,
        targetCount: document.querySelectorAll('#display-pause').length }; })()`) };
    const restoreReceiveSection = await evaluate(`(() => {
      const trigger = window.__settingsIndexTargets.trigger;
      trigger.focus(); trigger.click();
      if (trigger.id === 'phone-menu-btn') document.getElementById('phone-menu-settings-btn').click();
      document.querySelector('.settings-section-tab[data-settings-section="receive"]').click();
      const stored = localStorage.getItem('saturn.ui.settings.lastSection');
      document.getElementById('setup-close-btn').click();
      return { stored, menuClosed: document.getElementById('setup-menu').hidden,
        focusReturned: document.activeElement === trigger }; })()`);
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let reloaded = false;
    for (let attempt = 0; attempt < 160; attempt++) {
      await pause(50);
      reloaded = await evaluate(`document.readyState === 'complete' &&
        document.querySelectorAll('#settings-section-tabs .settings-section-tab').length === 11`).catch(() => false);
      if (reloaded) break;
    }
    if (!reloaded) throw new Error(`${scenario.name}: Settings did not initialize after reload`);
    await evaluate(`(() => { localStorage.setItem('saturn.remote.layout', 'desktop');
      window.applyLayout('desktop', false, false); return true; })()`);
    const persisted = await evaluate(`(() => {
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
      if (!trigger) {
        trigger = document.getElementById('phone-menu-btn');
        if (!visible(trigger)) throw Error('Visible Settings trigger missing after reload');
        trigger.focus(); trigger.click();
        const route = document.getElementById('phone-menu-settings-btn');
        if (!visible(route)) throw Error('Phone Settings route missing after reload');
        route.click();
      } else { trigger.focus(); trigger.click(); }
      const selected = document.querySelector('.settings-section-tab[aria-selected="true"]');
      const beforeClose = { stored: localStorage.getItem('saturn.ui.settings.lastSection'),
        selected: selected?.dataset.settingsSection, label: document.getElementById('settings-section-page').getAttribute('aria-label') };
      document.getElementById('setup-close-btn').click();
      return { ...beforeClose, menuClosed: document.getElementById('setup-menu').hidden,
        layout: document.documentElement.dataset.layout,
        focusReturned: document.activeElement === trigger, focus: document.activeElement?.id,
        floorCount: document.querySelectorAll('#display-spectrum-floor').length,
        volumeCount: document.querySelectorAll('#rx-volume').length,
        runtimeError: window.saturnDiagLastError || null }; })()`);
    // Exercise the existing TX presentation hook without changing radio state or
    // requesting transmit. This checks DOM semantics and viewport placement.
    const txVisual = await evaluate(`(() => {
      const hook = window.updateTxAppBarHooks;
      const api = window.SaturnRemoteNextBundle?.SaturnRemoteNext;
      const badge = document.getElementById('app-tx-state-badge');
      const bar = document.getElementById('tx-on-air-bar');
      const live = document.getElementById('tx-state-live');
      if (typeof hook !== 'function' || !api || !badge || !bar || !live) return { missing: true };
      const states = ['locked', 'disarmed', 'armed', 'engaging', 'transmitting', 'fault'].map(state => {
        hook(state);
        return { state, badge: badge.textContent, ariaLabel: badge.getAttribute('aria-label'),
          visual: badge.dataset.txVisualState, announcement: live.textContent,
          barHidden: bar.hidden, expectedBadge: api.TX_VISUAL_STATE_BADGE_TEXT[state],
          expectedAnnouncement: api.TX_VISUAL_STATE_ANNOUNCEMENT[state] };
      });
      const observer = new MutationObserver(() => {});
      observer.observe(live, { childList: true, characterData: true, subtree: true });
      hook('transmitting');
      observer.takeRecords();
      hook('transmitting');
      const duplicateAnnouncementMutations = observer.takeRecords().length;
      observer.disconnect();
      const geometry = () => {
        const r = bar.getBoundingClientRect();
        const hit = document.elementFromPoint(innerWidth / 2, 1);
        return { x: r.x, y: r.y, width: r.width, height: r.height,
          viewportWidth: innerWidth, position: getComputedStyle(bar).position,
          visible: !bar.hidden && bar.getClientRects().length > 0,
          above: hit === bar || bar.contains(hit), hit: hit?.id || hit?.className || hit?.tagName };
      };
      const bare = geometry();
      const visible = e => !!e && !e.closest('[hidden]') && e.getClientRects().length > 0;
      let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
      if (!trigger) {
        trigger = document.getElementById('phone-menu-btn');
        trigger.focus(); trigger.click();
        document.getElementById('phone-menu-settings-btn').click();
      } else { trigger.focus(); trigger.click(); }
      hook('transmitting');
      const overSheet = geometry();
      document.getElementById('setup-close-btn').click();
      hook('locked');
      return { states, liveRole: live.getAttribute('role'), ariaLive: live.getAttribute('aria-live'),
        ariaAtomic: live.getAttribute('aria-atomic'), duplicateAnnouncementMutations,
        bare, overSheet, hiddenAfterLock: bar.hidden }; })()`);
    const failures = [];
    if (!opened.menuOpen || opened.layout !== 'desktop' || opened.sectionCount !== 11 || opened.indexActive !== 'true' ||
      !opened.pageVisible || opened.sourceCounts.floor !== 1 || opened.sourceCounts.volume !== 1)
      failures.push('settings-index-not-populated-or-source-duplicated');
    if (navigation.clicked.selected !== 'receive' || navigation.clicked.label !== 'Receive settings' ||
      navigation.clicked.stored !== 'receive' || navigation.right.selected !== 'transmit' ||
      navigation.right.focus !== 'transmit' || navigation.left.selected !== 'receive' ||
      navigation.left.focus !== 'receive' || navigation.end.selected !== 'about' ||
      navigation.home.selected !== 'display' || navigation.displayRows < 5)
      failures.push('settings-section-switch-or-keyboard-navigation-failed');
    if (!legacy.found || legacy.indexActive !== 'false' || !legacy.panelVisible ||
      !legacy.pageHidden || !legacy.returnVisible || !legacy.targetVisible ||
      legacy.focus !== 'display-spectrum-floor' || !legacy.sameTarget || legacy.targetCount !== 1)
      failures.push('legacy-setting-did-not-reveal-original-control');
    if (back.indexActive !== 'true' || !back.pageVisible || back.selected !== 'display' ||
      back.focus !== 'display' || !back.returnHidden)
      failures.push('back-to-index-did-not-restore-section-and-focus');
    if (indexInventory.direct.length < 4 ||
      !['receive.volume', 'receive.filterLow', 'receive.filterHigh', 'radio.antenna']
        .every(id => indexInventory.direct.includes(id)) ||
      indexInventory.direct.some(id => id.startsWith('transmit.')) ||
      !indexInventory.navigation.includes('display.pause'))
      failures.push('settings-direct-control-inventory-unexpected');
    for (const probe of proxies) {
      if (probe.missing || probe.before.count !== 1 || probe.before.proxyCount !== 1 ||
        probe.before.target !== probe.before.proxy || probe.before.disabled !== probe.before.proxyDisabled ||
        !probe.forward.requested ||
        !probe.forward.observed.some(event => event.type === 'input' && event.value === probe.forward.requested) ||
        !probe.forward.observed.some(event => event.type === 'change') ||
        !probe.forward.synced || !probe.reverse.synced ||
        !probe.disabledMirrored || !probe.disabledRestored || !probe.sameSource)
        failures.push(`settings-proxy-sync-${probe.entryId}`);
    }
    // Pause is a direct safe action proxy: the original handler runs once while
    // Settings stays open, unlike navigation-only rows that reveal the source.
    if (!outside.found || outside.stored !== 'display' || outside.menuClosed ||
      outside.sourceClicks !== 1 || !outside.sameTarget || outside.targetCount !== 1)
      failures.push('outside-action-proxy-did-not-run-original-handler');
    if (restoreReceiveSection.stored !== 'receive' || !restoreReceiveSection.menuClosed ||
      !restoreReceiveSection.focusReturned) failures.push('settings-section-restore-before-reload-failed');
    if (persisted.stored !== 'receive' || persisted.selected !== 'receive' || persisted.layout !== 'desktop' ||
      persisted.label !== 'Receive settings' || !persisted.menuClosed ||
      !persisted.focusReturned || persisted.floorCount !== 1 || persisted.volumeCount !== 1)
      failures.push('last-section-or-close-focus-failed-after-reload');
    if (txVisual.missing || txVisual.liveRole !== 'status' || txVisual.ariaLive !== 'assertive' ||
      txVisual.ariaAtomic !== 'true' || txVisual.duplicateAnnouncementMutations !== 0 ||
      txVisual.states.some(item => item.badge !== item.expectedBadge || item.ariaLabel !== item.expectedBadge ||
        item.visual !== (item.state === 'engaging' ? 'armed' : item.state) ||
        item.announcement !== item.expectedAnnouncement ||
        item.barHidden !== (item.state !== 'transmitting')) || !txVisual.hiddenAfterLock)
      failures.push('tx-badge-or-live-region-transition-failed');
    for (const [name, geometry] of [['bare', txVisual.bare], ['over-sheet', txVisual.overSheet]]) {
      if (!geometry || !geometry.visible || geometry.position !== 'fixed' ||
        Math.abs(geometry.x) > 1 || Math.abs(geometry.y) > 1 ||
        Math.abs(geometry.width - geometry.viewportWidth) > 1 || Math.abs(geometry.height - 3) > 0.5 ||
        !geometry.above) failures.push(`tx-on-air-bar-${name}-geometry-or-stacking`);
    }
    const report = { scenario: scenario.name, opened, navigation, legacy, back, indexInventory,
      proxies, outside, restoreReceiveSection, persisted, txVisual,
      failures, ok: failures.length === 0 };
    settingsIndexReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || '11 sections, navigation, source routing, persistence, and focus'}`);
  }
  for (const scenario of exactEntryScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let ready = false;
    for (let attempt = 0; attempt < 160; attempt++) {
      await pause(50);
      ready = await evaluate(`document.readyState === 'complete' &&
        document.querySelectorAll('.exact-slider-value').length === 6`).catch(() => false);
      if (ready) break;
    }
    if (!ready) throw new Error(`${scenario.name}: full RX exact-entry runtime did not initialize`);
    const setup = await evaluate(`(() => {
      localStorage.setItem('saturn.remote.layout', 'desktop');
      window.applyLayout('desktop', false, false);
      const panel = document.querySelector('[data-phone-panel="audio"]');
      if (panel?.dataset.phoneCollapsed === 'true') panel.querySelector('.panel-toggle-btn')?.click();
      const readout = document.getElementById('rx-volume-readout');
      readout.scrollIntoView({ block: 'center', behavior: 'instant' });
      return { layout: document.documentElement.dataset.layout,
        count: document.querySelectorAll('.exact-slider-value').length,
        audioCollapsed: panel?.dataset.phoneCollapsed,
        volumeVisible: !readout.closest('[hidden]') && readout.getClientRects().length > 0 }; })()`);
    const controls = await evaluate(`(async () => {
      const specs = [
        ['rx-volume-readout', 'rx-volume'],
        ['rx-ssql-readout', 'rx-ssql-threshold'],
        ['agc-gain-readout', 'agc-gain'],
        ['rx-nb-threshold-readout', 'rx-nb-threshold'],
        ['filter-low-readout', 'filter-low'],
        ['filter-high-readout', 'filter-high'],
      ];
      const key = (node, name) => node.dispatchEvent(new KeyboardEvent('keydown',
        { key: name, bubbles: true, cancelable: true }));
      const nextFrame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const results = [];
      for (const [readoutId, sliderId] of specs) {
        const readout = document.getElementById(readoutId);
        const slider = document.getElementById(sliderId);
        const control = readout.closest('[data-control-context]');
        if (control?.hidden) {
          document.querySelector('#radio-context-rail .context-tab[data-control-context="' +
            control.dataset.controlContext + '"]')?.click();
        }
        const originalDisabled = slider.disabled;
        const before = { value: slider.value, text: readout.textContent.trim(),
          role: readout.getAttribute('role'), tabindex: readout.tabIndex,
          ariaLabel: readout.getAttribute('aria-label'), disabled: originalDisabled,
          visible: !readout.closest('[hidden]') && readout.getClientRects().length > 0,
          display: getComputedStyle(readout).display,
          hiddenAncestor: readout.closest('[hidden]')?.id || readout.closest('[hidden]')?.className || null,
          parentDisplay: getComputedStyle(readout.parentElement).display };
        const events = [];
        slider.addEventListener('input', () => events.push('input'));
        slider.addEventListener('change', () => events.push('change'));
        readout.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true }));
        const input = readout.nextElementSibling?.matches('.exact-slider-input')
          ? readout.nextElementSibling : null;
        const openState = { input: !!input, hidden: readout.hidden,
          focusInput: document.activeElement === input };
        const opened = openState.input && openState.hidden && openState.focusInput;
        const step = Number(slider.step) || 1;
        const current = Number(slider.value);
        const candidate = current + step <= Number(slider.max) ? current + step : current - step;
        if (input) { input.value = String(candidate); key(input, 'Enter'); }
        await nextFrame();
        const numericText = Number(readout.textContent.match(/-?\\d+(?:\\.\\d+)?/)?.[0]);
        const committed = { value: slider.value, text: readout.textContent.trim(), numericText,
          inputRemoved: !readout.nextElementSibling?.matches('.exact-slider-input'),
          readoutVisible: !readout.hidden, focusReturned: document.activeElement === readout,
          events: events.slice() };
        const eventCountBeforeCancel = events.length;
        readout.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true }));
        const cancelInput = readout.nextElementSibling?.matches('.exact-slider-input')
          ? readout.nextElementSibling : null;
        if (cancelInput) { cancelInput.value = String(current); key(cancelInput, 'Escape'); }
        const cancelled = { opened: !!cancelInput, value: slider.value, text: readout.textContent.trim(),
          inputRemoved: !readout.nextElementSibling?.matches('.exact-slider-input'),
          focusReturned: document.activeElement === readout, eventsAdded: events.length - eventCountBeforeCancel };
        readout.focus(); key(readout, 'Enter');
        const enterInput = readout.nextElementSibling?.matches('.exact-slider-input')
          ? readout.nextElementSibling : null;
        if (enterInput) key(enterInput, 'Escape');
        readout.focus(); key(readout, ' ');
        const spaceInput = readout.nextElementSibling?.matches('.exact-slider-input')
          ? readout.nextElementSibling : null;
        if (spaceInput) key(spaceInput, 'Escape');
        slider.disabled = true;
        readout.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, cancelable: true }));
        key(readout, 'Enter'); key(readout, ' ');
        const disabledBlocked = !readout.nextElementSibling?.matches('.exact-slider-input');
        slider.disabled = originalDisabled;
        results.push({ readoutId, sliderId, before, opened, openState, candidate, committed, cancelled,
          keyboard: { enterOpened: !!enterInput, spaceOpened: !!spaceInput }, disabledBlocked,
          finalValue: slider.value, sourceCount: document.querySelectorAll('#' + sliderId).length });
      }
      return results;
    })()`);
    const mousePoint = await evaluate(`(() => {
      const readout = document.getElementById('rx-volume-readout');
      readout.scrollIntoView({ block: 'center', behavior: 'instant' });
      const r = readout.getBoundingClientRect();
      const x = r.x + r.width / 2, y = r.y + r.height / 2;
      const hit = document.elementFromPoint(x, y);
      return { x, y, hit: hit?.id || null, visible: r.width > 0 && r.height > 0 }; })()`);
    for (const clickCount of [1, 2]) {
      await call('Input.dispatchMouseEvent', { type: 'mousePressed', x: mousePoint.x, y: mousePoint.y,
        button: 'left', clickCount });
      await call('Input.dispatchMouseEvent', { type: 'mouseReleased', x: mousePoint.x, y: mousePoint.y,
        button: 'left', clickCount });
    }
    const mouseDoubleClick = await evaluate(`(() => {
      const readout = document.getElementById('rx-volume-readout');
      const input = readout.nextElementSibling?.matches('.exact-slider-input')
        ? readout.nextElementSibling : null;
      const result = { opened: !!input, focused: document.activeElement === input };
      input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
      return result; })()`);
    let touch = null;
    if (scenario.width === 390) {
      const point = await evaluate(`(() => {
        const readout = document.getElementById('rx-volume-readout');
        readout.scrollIntoView({ block: 'center', behavior: 'instant' });
        const r = readout.getBoundingClientRect();
        const x = r.x + r.width / 2, y = r.y + r.height / 2;
        const hit = document.elementFromPoint(x, y);
        return { x, y, hit: hit?.id || null, visible: r.width > 0 && r.height > 0 }; })()`);
      const tap = async id => {
        await call('Input.dispatchTouchEvent', { type: 'touchStart',
          touchPoints: [{ x: point.x, y: point.y, id, radiusX: 1, radiusY: 1 }] });
        await call('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
      };
      await tap(41);
      await pause(70);
      const afterFirst = await evaluate(`!!document.querySelector('#rx-volume-readout + .exact-slider-input')`);
      await tap(42);
      await pause(70);
      const afterSecond = await evaluate(`(() => {
        const readout = document.getElementById('rx-volume-readout');
        const input = readout.nextElementSibling?.matches('.exact-slider-input')
          ? readout.nextElementSibling : null;
        const result = { opened: !!input, focused: document.activeElement === input };
        input?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
        return result; })()`);
      touch = { point, afterFirst, afterSecond };
    }
    const failures = [];
    if (setup.layout !== 'desktop' || setup.count !== 6 || !setup.volumeVisible || setup.audioCollapsed === 'true')
      failures.push('rx-exact-entry-not-ready-or-visible');
    for (const control of controls) {
      if (control.before.role !== 'button' || control.before.tabindex !== 0 || !control.before.ariaLabel ||
        !control.before.visible ||
        !control.opened || !control.committed.inputRemoved || !control.committed.readoutVisible ||
        !control.committed.focusReturned ||
        Math.abs(Number(control.committed.value) - control.candidate) > 0.01 ||
        Math.abs(control.committed.numericText - control.candidate) > 0.11 ||
        !control.committed.events.includes('input') || !control.committed.events.includes('change'))
        failures.push(`rx-exact-commit-${control.sliderId}`);
      if (!control.cancelled.opened || !control.cancelled.inputRemoved ||
        !control.cancelled.focusReturned || control.cancelled.value !== control.committed.value ||
        control.cancelled.text !== control.committed.text || control.cancelled.eventsAdded !== 0)
        failures.push(`rx-exact-cancel-${control.sliderId}`);
      if (!control.keyboard.enterOpened || !control.keyboard.spaceOpened ||
        !control.disabledBlocked || control.sourceCount !== 1)
        failures.push(`rx-exact-keyboard-or-disabled-${control.sliderId}`);
    }
    if (touch && (!touch.point.visible || touch.point.hit !== 'rx-volume-readout' ||
      touch.afterFirst || !touch.afterSecond.opened || !touch.afterSecond.focused))
      failures.push('rx-exact-touch-double-tap');
    if (!mousePoint.visible || mousePoint.hit !== 'rx-volume-readout' ||
      !mouseDoubleClick.opened || !mouseDoubleClick.focused)
      failures.push('rx-exact-physical-double-click');
    const report = { scenario: scenario.name, setup, controls, mousePoint, mouseDoubleClick,
      touch, failures, ok: failures.length === 0 };
    exactEntryReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'six exact values, cancel, keyboard, disabled, and touch'}`);
  }
  for (const scenario of systemRouteScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let ready = false;
    for (let attempt = 0; attempt < 160; attempt++) {
      await pause(50);
      ready = await evaluate(`document.readyState === 'complete' &&
        typeof window.updateUi === 'function' && !!document.getElementById('system-go-offline-btn')`).catch(() => false);
      if (ready) break;
    }
    if (!ready) throw new Error(`${scenario.name}: System runtime did not initialize`);
    const result = await evaluate(`(async () => {
      const hasState = typeof state !== 'undefined';
      const hasDisconnect = typeof window.disconnectBridge === 'function';
      if (!hasState || !hasDisconnect) return { missing: true, hasState, hasDisconnect };
      localStorage.setItem('saturn.remote.layout', 'desktop');
      window.applyLayout('desktop', false, false);
      const goLiveButton = document.getElementById('go-live-btn');
      const systemGoLiveButton = document.getElementById('system-go-live-btn');
      const systemButton = document.getElementById('system-go-offline-btn');
      const systemMenu = document.querySelector('.header-diagnostics-menu');
      const visible = node => !!node && !node.closest('[hidden]') &&
        !node.closest('details:not([open])') && node.getClientRects().length > 0;
      const hit = node => {
        if (!visible(node)) return false;
        node.scrollIntoView({ block: 'nearest', behavior: 'instant' });
        const r = node.getBoundingClientRect();
        const target = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return target === node || node.contains(target);
      };
      const originalConfirm = window.confirm;
      const originalDisconnect = window.disconnectBridge;
      const calls = [];
      const prompts = [];
      let disconnectPromise = null;
      let disconnectError = null;
      window.disconnectBridge = (...args) => {
        calls.push('disconnectBridge');
        disconnectPromise = Promise.resolve(originalDisconnect(...args))
          .catch(error => { disconnectError = String(error); });
        return disconnectPromise;
      };
      const offline = { connected: state.connected,
        goLiveVisible: ${scenario.width} < 600 ? !systemGoLiveButton.hidden : visible(goLiveButton),
        goLiveText: goLiveButton.textContent.trim(), systemHidden: systemButton.hidden,
        systemGoLiveText: systemGoLiveButton.textContent.trim(),
        goLiveCount: document.querySelectorAll('#go-live-btn').length,
        systemCount: document.querySelectorAll('#system-go-offline-btn').length };
      state.connected = true;
      state.bridgeReady = true;
      updateUi();
      const live = { connected: state.connected, ready: state.bridgeReady,
        goLiveShown: !goLiveButton.hidden, goLiveText: goLiveButton.textContent.trim(),
        goLiveRendered: visible(goLiveButton), systemShown: !systemButton.hidden,
        systemGoLiveHidden: systemGoLiveButton.hidden };
      let route;
      if (${scenario.width} < 600) {
        const trigger = document.getElementById('phone-menu-btn');
        trigger.focus(); trigger.click();
        const sheet = document.getElementById('phone-menu-sheet');
        const systemRoute = document.getElementById('phone-menu-system-btn');
        const routeVisible = visible(systemRoute);
        systemRoute.click();
        const host = document.getElementById('phone-menu-system-host');
        route = { phone: true, routeVisible, sheetOpen: !sheet.hidden,
          hostVisible: visible(host), sameMenu: systemMenu.parentElement === host,
          systemVisible: visible(systemButton), systemHit: hit(systemButton) };
      } else {
        const details = document.querySelector('.header-diagnostics');
        details.querySelector('summary').click();
        route = { phone: false, detailsOpen: details.open,
          sameMenu: systemMenu.parentElement === details,
          systemVisible: visible(systemButton), systemHit: hit(systemButton) };
      }
      window.confirm = message => { prompts.push(message); return false; };
      if (${scenario.width} >= 600) goLiveButton.click();
      const mainCancelled = { connected: state.connected, ready: state.bridgeReady,
        disconnectCalls: calls.length, promptCount: prompts.length,
        buttonText: goLiveButton.textContent.trim() };
      systemButton.click();
      const cancelled = { connected: state.connected, ready: state.bridgeReady,
        disconnectCalls: calls.length, promptCount: prompts.length,
        goLiveShown: !goLiveButton.hidden, goLiveText: goLiveButton.textContent.trim(),
        systemShown: !systemButton.hidden };
      window.confirm = message => { prompts.push(message); return true; };
      systemButton.click();
      if (disconnectPromise) await disconnectPromise;
      const accepted = { connected: state.connected, ready: state.bridgeReady,
        disconnectCalls: calls.length, promptCount: prompts.length,
        goLiveVisible: ${scenario.width} < 600 ? visible(systemGoLiveButton) : visible(goLiveButton),
        goLiveText: goLiveButton.textContent.trim(),
        systemHidden: systemButton.hidden, sameGoLiveButton: goLiveButton === document.getElementById('go-live-btn'),
        disconnectError };
      window.confirm = originalConfirm;
      window.disconnectBridge = originalDisconnect;
      return { offline, live, route, mainCancelled, cancelled, accepted, prompts,
        disconnectOverridden: window.disconnectBridge === originalDisconnect,
        layout: document.documentElement.dataset.layout };
    })()`);
    const failures = [];
    if (result.missing || result.layout !== 'desktop' || result.offline?.connected ||
      !result.offline?.goLiveVisible || result.offline?.goLiveText !== 'Go Live' ||
      result.offline?.systemGoLiveText !== 'Go Live' ||
      !result.offline?.systemHidden || result.offline?.goLiveCount !== 1 || result.offline?.systemCount !== 1)
      failures.push('offline-go-live-or-system-baseline-failed');
    if (!result.live?.connected || !result.live?.ready || !result.live?.goLiveShown ||
      result.live?.goLiveText !== 'Go Offline' || !result.live?.systemGoLiveHidden ||
      (scenario.width >= 600 && !result.live?.goLiveRendered) ||
      !result.live?.systemShown || !result.route?.sameMenu || !result.route?.systemVisible ||
      !result.route?.systemHit || (scenario.width < 600 &&
        (!result.route?.routeVisible || !result.route?.sheetOpen || !result.route?.hostVisible)) ||
      (scenario.width >= 600 && !result.route?.detailsOpen))
      failures.push('live-system-action-not-reachable');
    if (scenario.width >= 600 && (!result.mainCancelled?.connected || !result.mainCancelled?.ready ||
      result.mainCancelled?.disconnectCalls !== 0 || result.mainCancelled?.promptCount !== 1 ||
      result.mainCancelled?.buttonText !== 'Go Offline')) failures.push('main-go-offline-cancel');
    const expectedCancelPrompts = scenario.width >= 600 ? 2 : 1;
    if (!result.cancelled?.connected || !result.cancelled?.ready ||
      result.cancelled?.disconnectCalls !== 0 || result.cancelled?.promptCount !== expectedCancelPrompts ||
      !result.cancelled?.goLiveShown || result.cancelled?.goLiveText !== 'Go Offline' ||
      !result.cancelled?.systemShown)
      failures.push('cancel-disconnected-or-changed-live-state');
    if (result.accepted?.connected || result.accepted?.ready ||
      result.accepted?.disconnectCalls !== 1 || result.accepted?.promptCount !== expectedCancelPrompts + 1 ||
      !result.accepted?.goLiveVisible || result.accepted?.goLiveText !== 'Go Live' ||
      !result.accepted?.systemHidden || !result.accepted?.sameGoLiveButton ||
      result.accepted?.disconnectError ||
      !result.disconnectOverridden || result.prompts?.some(prompt => !prompt.includes('disconnect')))
      failures.push('accept-did-not-invoke-single-disconnect-and-restore-go-live');
    const report = { scenario: scenario.name, result, failures, ok: failures.length === 0 };
    systemRouteReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'offline Go Live, live System route, cancel/accept disconnect'}`);
  }
  for (const scenario of mainScreenScenarios) {
    const tier = scenario.width < 600 ? 'phone' : 'desktop';
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    const waitForMainScreen = async () => {
      for (let attempt = 0; attempt < 160; attempt++) {
        await pause(50);
        if (await evaluate(`document.readyState === 'complete' &&
          document.querySelectorAll('#settings-section-tabs .settings-section-tab').length === 11 &&
          !!document.getElementById('main-screen-pins')`).catch(() => false)) return;
      }
      throw new Error(`${scenario.name}: main-screen runtime did not initialize`);
    };
    await waitForMainScreen();
    await evaluate(`(() => {
      localStorage.removeItem('saturn.ui.layout.phone');
      localStorage.removeItem('saturn.ui.layout.desktop');
      localStorage.setItem('saturn.remote.layout', 'desktop');
      return true;
    })()`);
    await call('Page.reload', { ignoreCache: true });
    await waitForMainScreen();
    const initial = await evaluate(`(() => {
      window.applyLayout('desktop', false, false);
      const visible = node => !!node && !node.closest('[hidden]') && node.getClientRects().length > 0;
      let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
      if (trigger) trigger.click();
      else {
        document.getElementById('phone-menu-btn').click();
        trigger = document.getElementById('phone-menu-settings-btn');
        trigger.click();
      }
      document.querySelector('.settings-section-tab[data-settings-section="interface"]').click();
      const selector = document.querySelector('.settings-layout-tier');
      const check = id => [...document.querySelectorAll('[data-layout-pin]')].find(node => node.dataset.layoutPin === id);
      const safety = (window.SaturnRemoteNextBundle?.SaturnRemoteNext?.LAYOUT_SAFETY_PINNED_IDS || [])
        .map(id => ({ id, found: !!check(id), disabled: check(id)?.disabled, checked: check(id)?.checked }));
      return { trigger: trigger?.id, menuOpen: !document.getElementById('setup-menu').hidden,
        selector: selector?.value, options: [...selector.options].map(option => option.value),
        avg: check('display.averageToolbar')?.checked, peak: check('display.peakToolbar')?.checked,
        safety, layout: document.documentElement.dataset.layout };
    })()`);
    const changed = await evaluate(`(() => {
      const selector = () => document.querySelector('.settings-layout-tier');
      const check = id => [...document.querySelectorAll('[data-layout-pin]')].find(node => node.dataset.layoutPin === id);
      const toolbar = () => [...document.querySelector('.display-pill-row').children].map(node => node.id);
      const state = id => { const node = document.getElementById(id); return {
        display: getComputedStyle(node).display, rects: node.getClientRects().length,
        hidden: node.dataset.mainLayoutHidden, pinned: node.dataset.mainLayoutPinned,
      }; };
      const current = '${tier}';
      const other = current === 'phone' ? 'desktop' : 'phone';
      const before = { avg: state('spectrum-average-toolbar-btn'), peak: state('spectrum-peak-toolbar-btn') };
      for (const id of ['display.averageToolbar', 'display.peakToolbar']) {
        if (!check(id).checked) check(id).click();
      }
      const bothOn = { avg: state('spectrum-average-toolbar-btn'), peak: state('spectrum-peak-toolbar-btn') };
      for (const id of ['display.averageToolbar', 'display.peakToolbar']) check(id).click();
      const bothOff = { avg: state('spectrum-average-toolbar-btn'), peak: state('spectrum-peak-toolbar-btn') };
      for (const id of ['display.averageToolbar', 'display.peakToolbar']) check(id).click();
      const restored = { avg: state('spectrum-average-toolbar-btn'), peak: state('spectrum-peak-toolbar-btn') };
      const currentStorage = localStorage.getItem('saturn.ui.layout.' + current);
      selector().value = other;
      selector().dispatchEvent(new Event('change', { bubbles: true }));
      const otherTier = { selected: selector().value, avg: check('display.averageToolbar')?.checked,
        peak: check('display.peakToolbar')?.checked, storage: localStorage.getItem('saturn.ui.layout.' + other) };
      selector().value = current;
      selector().dispatchEvent(new Event('change', { bubbles: true }));
      const back = { selected: selector().value, avg: check('display.averageToolbar')?.checked,
        peak: check('display.peakToolbar')?.checked, storagePreserved: currentStorage === localStorage.getItem('saturn.ui.layout.' + current) };
      const sliderPin = check('receive.nrLevel');
      const sliderBefore = { checked: sliderPin?.checked, sourceCount: document.querySelectorAll('#setup-dsp-nr-level').length };
      if (!sliderPin.checked) sliderPin.click();
      const row = [...document.querySelectorAll('#main-screen-pins .main-screen-pin')]
        .find(node => node.textContent.includes('Noise reduction level'));
      const proxy = row?.querySelector('input[type="range"]');
      const source = document.getElementById('setup-dsp-nr-level');
      if (proxy && source) {
        proxy.value = String(Math.max(Number(proxy.min), Math.min(Number(proxy.max), Number(proxy.value) - Number(proxy.step || 1))));
        proxy.dispatchEvent(new Event('input', { bubbles: true }));
      }
      const sliderAfter = { checked: check('receive.nrLevel')?.checked,
        trayVisible: !!row && !row.closest('[hidden]') && row.getClientRects().length > 0,
        proxy: !!proxy, proxyValue: proxy?.value, sourceValue: source?.value,
        sourceCount: document.querySelectorAll('#setup-dsp-nr-level').length };
      const mox = document.getElementById('mox-btn');
      const moxHome = mox.parentElement;
      if (!check('transmit.mox').checked) check('transmit.mox').click();
      const pinnedMox = { checked: check('transmit.mox').checked,
        originalNode: document.getElementById('mox-btn') === mox,
        inTray: !!mox.closest('#main-screen-pins'),
        visible: !!mox.getClientRects().length && !mox.closest('[hidden]'),
        count: document.querySelectorAll('#mox-btn').length, homeHidden: moxHome.hidden };
      check('transmit.mox').click();
      const unpinnedMox = { checked: check('transmit.mox').checked, home: mox.parentElement === moxHome,
        count: document.querySelectorAll('#mox-btn').length, homeHidden: moxHome.hidden };
      check('transmit.mox').click();
      window.applyLayout('phone', false, false);
      const separatePhoneMox = { home: mox.parentElement === moxHome,
        count: document.querySelectorAll('#mox-btn').length, homeHidden: moxHome.hidden };
      window.applyLayout('desktop', false, false);
      window.applyMainScreenLayout();
      const responsiveMox = { inTray: !!mox.closest('#main-screen-pins'),
        visible: !!mox.getClientRects().length && !mox.closest('[hidden]') };
      document.querySelector('.settings-section-tab[data-settings-section="interface"]').click();
      const orderBefore = [...document.querySelectorAll('[data-toolbar-order]')].map(node => node.dataset.toolbarOrder);
      const up = document.querySelector('[data-toolbar-order="display.peakToolbar"] button[aria-label$=" up"]');
      window.__mainKeyEvents = [];
      document.addEventListener('keydown', event => window.__mainKeyEvents.push({ key: event.key, target: event.target?.getAttribute('aria-label') }), true);
      up?.addEventListener('click', () => window.__mainKeyEvents.push({ click: 'up' }));
      up?.focus();
      return { before, bothOn, bothOff, restored, otherTier, back,
        sliderBefore, sliderAfter, pinnedMox, unpinnedMox, separatePhoneMox, responsiveMox,
        orderBefore, toolbarBefore: toolbar(),
        focusedUp: document.activeElement === up, upDisabled: up?.disabled,
        storage: localStorage.getItem('saturn.ui.layout.' + current) };
    })()`);
    const editorPng = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}-editor.png`), Buffer.from(editorPng.data, 'base64'));
    // Pin changes schedule a focus-restoring render; wait for it to settle,
    // then focus the Move button immediately before the physical key press.
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
    const spaceUpFocused = await evaluate(`(() => {
      const up = document.querySelector('[data-toolbar-order="display.peakToolbar"] button[aria-label$=" up"]');
      up?.focus();
      return document.activeElement === up && !up.disabled;
    })()`);
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: ' ', code: 'Space', text: ' ', windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32 });
    const moved = await evaluate(`(() => {
      const list = () => [...document.querySelectorAll('[data-toolbar-order]')].map(node => node.dataset.toolbarOrder);
      const afterUp = list();
      const toolbarAfterUp = [...document.querySelector('.display-pill-row').children].map(node => node.id);
      const down = document.querySelector('[data-toolbar-order="display.peakToolbar"] button[aria-label$=" down"]');
      down?.focus();
      return { afterUp, toolbarAfterUp, focusedDown: document.activeElement === down,
        downDisabled: down?.disabled, keyEvents: window.__mainKeyEvents };
    })()`);
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: ' ', code: 'Space', text: ' ', windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: ' ', code: 'Space', windowsVirtualKeyCode: 32, nativeVirtualKeyCode: 32 });
    const afterDown = await evaluate(`(() => ({
      order: [...document.querySelectorAll('[data-toolbar-order]')].map(node => node.dataset.toolbarOrder),
      toolbar: [...document.querySelector('.display-pill-row').children].map(node => node.id),
      keyEvents: window.__mainKeyEvents,
      storage: localStorage.getItem('saturn.ui.layout.${tier}'),
    }))()`);
    const focusedEnterUp = await evaluate(`(() => {
      const up = document.querySelector('[data-toolbar-order="display.peakToolbar"] button[aria-label$=" up"]');
      up?.focus();
      return document.activeElement === up && !up.disabled;
    })()`);
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', text: '\r',
      unmodifiedText: '\r', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    const enterUp = await evaluate(`(() => {
      const order = [...document.querySelectorAll('[data-toolbar-order]')].map(node => node.dataset.toolbarOrder);
      const down = document.querySelector('[data-toolbar-order="display.peakToolbar"] button[aria-label$=" down"]');
      down?.focus();
      return { order, focusedDown: document.activeElement === down && !down.disabled,
        toolbar: [...document.querySelector('.display-pill-row').children].map(node => node.id) };
    })()`);
    await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', text: '\r',
      unmodifiedText: '\r', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 });
    const enterDown = await evaluate(`(() => ({
      order: [...document.querySelectorAll('[data-toolbar-order]')].map(node => node.dataset.toolbarOrder),
      toolbar: [...document.querySelector('.display-pill-row').children].map(node => node.id),
      storage: localStorage.getItem('saturn.ui.layout.${tier}'),
      keyEvents: window.__mainKeyEvents,
    }))()`);
    await call('Page.reload', { ignoreCache: true });
    await waitForMainScreen();
    const reloaded = await evaluate(`(() => {
      window.applyLayout('desktop', false, false);
      const state = id => { const node = document.getElementById(id); return {
        display: getComputedStyle(node).display, hidden: node.dataset.mainLayoutHidden,
        pinned: node.dataset.mainLayoutPinned,
      }; };
      return { avg: state('spectrum-average-toolbar-btn'), peak: state('spectrum-peak-toolbar-btn'),
        toolbar: [...document.querySelector('.display-pill-row').children].map(node => node.id),
        sliderRow: [...document.querySelectorAll('#main-screen-pins .main-screen-pin')]
          .some(node => node.textContent.includes('Noise reduction level') && !!node.querySelector('input[type="range"]')),
        moxPinned: !!document.getElementById('mox-btn')?.closest('#main-screen-pins'),
        moxCount: document.querySelectorAll('#mox-btn').length,
        storage: localStorage.getItem('saturn.ui.layout.${tier}') };
    })()`);
    const screenPng = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}-reloaded.png`), Buffer.from(screenPng.data, 'base64'));
    const failures = [];
    const isShown = item => item?.display !== 'none' && item?.hidden !== 'true';
    const isHidden = item => item?.display === 'none' && item?.hidden === 'true';
    if (!initial.menuOpen || initial.selector !== tier || initial.layout !== 'desktop' ||
      initial.options.join(',') !== 'phone,tablet,desktop,wide') failures.push('tier-selector-or-settings-route');
    if (initial.safety.length !== 6 || initial.safety.some(item => !item.found || !item.disabled || !item.checked))
      failures.push('safety-pins-not-locked');
    if ((initial.avg && !isShown(changed.before.avg)) || (!initial.avg && !isHidden(changed.before.avg)) ||
      (initial.peak && !isShown(changed.before.peak)) || (!initial.peak && !isHidden(changed.before.peak)))
      failures.push('default-pin-visibility');
    if (!isShown(changed.bothOn.avg) || !isShown(changed.bothOn.peak) ||
      !isHidden(changed.bothOff.avg) || !isHidden(changed.bothOff.peak) ||
      !isShown(changed.restored.avg) || !isShown(changed.restored.peak))
      failures.push('avg-peak-pin-visibility');
    if (changed.otherTier.selected !== (tier === 'phone' ? 'desktop' : 'phone') ||
      changed.otherTier.storage !== null || changed.back.selected !== tier ||
      !changed.back.avg || !changed.back.peak || !changed.back.storagePreserved)
      failures.push('tier-selection-isolation');
    if (changed.sliderBefore.sourceCount !== 1 || !changed.sliderAfter.checked ||
      !changed.sliderAfter.trayVisible || !changed.sliderAfter.proxy ||
      changed.sliderAfter.proxyValue !== changed.sliderAfter.sourceValue ||
      changed.sliderAfter.sourceCount !== 1) failures.push('pinned-slider-proxy');
    if (!changed.pinnedMox.checked || !changed.pinnedMox.originalNode ||
      !changed.pinnedMox.inTray || !changed.pinnedMox.visible ||
      changed.pinnedMox.count !== 1 || !changed.pinnedMox.homeHidden ||
      changed.unpinnedMox.checked || !changed.unpinnedMox.home ||
      changed.unpinnedMox.count !== 1 || changed.unpinnedMox.homeHidden ||
      !changed.separatePhoneMox.home || changed.separatePhoneMox.count !== 1 ||
      changed.separatePhoneMox.homeHidden || !changed.responsiveMox.inTray ||
      !changed.responsiveMox.visible || !reloaded.moxPinned || reloaded.moxCount !== 1)
      failures.push('pinned-mox-original-control-or-persistence');
    const peak = 'display.peakToolbar';
    const beforePeak = changed.orderBefore.indexOf(peak);
    if (!spaceUpFocused || changed.upDisabled || beforePeak < 1 ||
      moved.afterUp.indexOf(peak) !== beforePeak - 1 ||
      moved.toolbarAfterUp.indexOf('spectrum-peak-toolbar-btn') >=
        moved.toolbarAfterUp.indexOf('spectrum-average-toolbar-btn'))
      failures.push('keyboard-move-up-or-dom-order');
    if (!moved.focusedDown || moved.downDisabled ||
      afterDown.order.indexOf(peak) !== beforePeak ||
      afterDown.toolbar.indexOf('spectrum-peak-toolbar-btn') <=
        afterDown.toolbar.indexOf('spectrum-average-toolbar-btn'))
      failures.push('keyboard-move-down-or-dom-order');
    if (!focusedEnterUp || enterUp.order.indexOf(peak) !== beforePeak - 1 ||
      enterUp.toolbar.indexOf('spectrum-peak-toolbar-btn') >=
        enterUp.toolbar.indexOf('spectrum-average-toolbar-btn') ||
      !enterUp.focusedDown || enterDown.order.indexOf(peak) !== beforePeak ||
      enterDown.toolbar.indexOf('spectrum-peak-toolbar-btn') <=
        enterDown.toolbar.indexOf('spectrum-average-toolbar-btn'))
      failures.push('enter-move-up-down-or-dom-order');
    if (!isShown(reloaded.avg) || !isShown(reloaded.peak) || !reloaded.sliderRow ||
      reloaded.storage !== afterDown.storage ||
      reloaded.toolbar.indexOf('spectrum-average-toolbar-btn') >=
        reloaded.toolbar.indexOf('spectrum-peak-toolbar-btn')) failures.push('reload-persistence');
    const report = { scenario: scenario.name, initial, changed, spaceUpFocused, moved, afterDown,
      focusedEnterUp, enterUp, enterDown, reloaded,
      failures, ok: failures.length === 0 };
    mainScreenReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'tier, pin visibility, toolbar keys, safety, slider, persistence'}`);
  }
  for (const scenario of restoreScenarios) {
    const tier = scenario.width < 600 ? 'phone' : 'desktop';
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    const ready = async () => {
      for (let attempt = 0; attempt < 160; attempt++) {
        await pause(50);
        if (await evaluate(`document.readyState === 'complete' &&
          document.querySelectorAll('#settings-section-tabs .settings-section-tab').length === 11 &&
          typeof window.applyTheme === 'function'`).catch(() => false)) return;
      }
      throw new Error(`${scenario.name}: restore runtime did not initialize`);
    };
    await ready();
    await evaluate(`(() => {
      for (const key of ['saturn.ui.theme', 'saturn.remote.theme',
        'saturn.ui.layout.phone', 'saturn.ui.layout.desktop',
        'saturn.ui.phone.rxOpen', 'saturn.ui.phone.txOpen']) localStorage.removeItem(key);
      localStorage.setItem('saturn.remote.layout', 'desktop');
      return true;
    })()`);
    await call('Page.reload', { ignoreCache: true });
    await ready();
    const result = await evaluate(`(async () => {
      window.applyLayout('desktop', false, false);
      const tier = '${tier}';
      const key = 'saturn.ui.layout.' + tier;
      const visible = node => !!node && !node.closest('[hidden]') && node.getClientRects().length > 0;
      const pin = id => [...document.querySelectorAll('[data-layout-pin]')].find(node => node.dataset.layoutPin === id);
      const openSettings = () => {
        let trigger = ['header-setup-btn', 'setup-menu-btn'].map(id => document.getElementById(id)).find(visible);
        if (trigger) trigger.click();
        else {
          document.getElementById('phone-menu-btn').click();
          trigger = document.getElementById('phone-menu-settings-btn');
          trigger.click();
        }
        return trigger?.id;
      };
      const section = name => document.querySelector('.settings-section-tab[data-settings-section="' + name + '"]').click();
      const button = text => [...document.querySelectorAll('#settings-section-page .settings-restore-footer button')]
        .find(node => node.textContent.trim() === text);
      const radio = () => ({ vfoA: state.vfoA, dds: state.dds, mode: state.mode,
        band: bandKeyForFrequency(state.vfoA), txPhase: state.txPhase,
        txEnabled: state.txEnabled, moxRequested: state.moxRequested,
        txBadge: document.getElementById('app-tx-state-badge')?.textContent?.trim() });
      const snapshot = () => ({ theme: document.documentElement.dataset.theme,
        themeKey: localStorage.getItem('saturn.ui.theme'), layoutKey: localStorage.getItem(key),
        avgPinned: window.SaturnRemoteNextBundle.SaturnRemoteNext.loadLayout(tier).pinned.includes('display.averageToolbar'),
        rxKey: localStorage.getItem('saturn.ui.phone.rxOpen'),
        txKey: localStorage.getItem('saturn.ui.phone.txOpen'),
        rxCollapsed: document.querySelector('[data-phone-panel="audio"]')?.dataset.phoneCollapsed,
        txCollapsed: document.querySelector('[data-phone-panel="tx"]')?.dataset.phoneCollapsed,
        radio: radio() });
      const trigger = openSettings();
      section('interface');
      state.vfoA = 7150000;
      state.dds = 7150000;
      state.mode = 'LSB';
      updateUi();
      applyTheme('light');
      const defaultAvg = pin('display.averageToolbar').checked;
      pin('display.averageToolbar').click();
      localStorage.setItem('saturn.ui.phone.rxOpen', 'false');
      localStorage.setItem('saturn.ui.phone.txOpen', 'true');
      loadResponsivePhoneDisclosure();
      syncPhonePanels();
      const before = snapshot();
      const globalButton = button('Restore all interface defaults');
      const prompts = [];
      const originalConfirm = window.confirm;
      let cancelled;
      let accepted;
      let receive;
      let operations = null;
      try {
        window.confirm = message => { prompts.push(message); return false; };
        globalButton?.click();
        cancelled = snapshot();
        window.confirm = message => { prompts.push(message); return true; };
        globalButton?.click();
        accepted = snapshot();
        applyTheme('light');
        if (pin('display.averageToolbar').checked === defaultAvg) pin('display.averageToolbar').click();
        localStorage.setItem('saturn.ui.phone.rxOpen', 'false');
        localStorage.setItem('saturn.ui.phone.txOpen', 'true');
        loadResponsivePhoneDisclosure();
        syncPhonePanels();
        const sectionBefore = snapshot();
        section('receive');
        const receiveButton = button('Restore receive defaults');
        receiveButton?.click();
        receive = { found: !!receiveButton, before: sectionBefore, after: snapshot() };
        if (tier === 'desktop') {
          section('interface');
          const drawerPin = pin('shell.operationsDrawer');
          const drawer = document.getElementById('operations-drawer');
          const memoryTarget = document.getElementById('operations-tab-memory');
          if (drawerPin?.checked) drawerPin.click();
          const hidden = { pinChecked: drawerPin?.checked,
            drawerHidden: drawer?.dataset.mainLayoutHidden,
            display: drawer ? getComputedStyle(drawer).display : null,
            sameTarget: memoryTarget === document.getElementById('operations-tab-memory') };
          section('memory');
          const row = document.querySelector('#settings-section-page [data-settings-entry="memory.operationsTab"]');
          row?.click();
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
          const revealed = { rowFound: !!row, menuClosed: document.getElementById('setup-menu').hidden,
            drawerReveal: drawer?.dataset.layoutReveal, drawerOpen: drawer?.dataset.open,
            display: drawer ? getComputedStyle(drawer).display : null,
            targetVisible: visible(memoryTarget), targetFocused: document.activeElement === memoryTarget,
            sameTarget: memoryTarget === document.getElementById('operations-tab-memory') };
          document.getElementById('header-setup-btn').click();
          const rehidden = { menuOpen: !document.getElementById('setup-menu').hidden,
            drawerReveal: drawer?.dataset.layoutReveal || null,
            display: drawer ? getComputedStyle(drawer).display : null,
            drawerHidden: drawer?.dataset.mainLayoutHidden };
          operations = { hidden, revealed, rehidden };
        }
      } finally { window.confirm = originalConfirm; }
      return { tier, trigger, globalButtonFound: !!globalButton, defaultAvg,
        before, cancelled, accepted, receive, operations, prompts,
        confirmRestored: window.confirm === originalConfirm };
    })()`);
    const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, `${scenario.name}.png`), Buffer.from(png.data, 'base64'));
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    const failures = [];
    if (!result.globalButtonFound || !result.trigger || !result.confirmRestored ||
      result.prompts.length !== 3 || result.prompts.some(prompt => !prompt.includes('Radio frequency, band and transmit state')))
      failures.push('restore-confirmation-route');
    if (result.before.theme !== 'light' || result.before.themeKey !== 'light' ||
      result.before.avgPinned === result.defaultAvg || result.before.rxKey !== 'false' ||
      result.before.txKey !== 'true' || result.before.radio.vfoA !== 7150000 ||
      result.before.radio.band !== '40m') failures.push('restore-nondefault-setup');
    if (!same(result.before, result.cancelled)) failures.push('cancel-changed-state');
    if (result.accepted.theme !== 'dark' || result.accepted.themeKey !== 'dark' ||
      result.accepted.layoutKey !== null || result.accepted.avgPinned !== result.defaultAvg ||
      result.accepted.rxKey !== null || result.accepted.txKey !== null ||
      !same(result.accepted.radio, result.before.radio)) failures.push('global-restore-state-or-safety');
    if (!result.receive.found || result.receive.before.theme !== 'light' ||
      result.receive.before.avgPinned === result.defaultAvg ||
      result.receive.after.rxKey !== null || result.receive.after.txKey !== 'true' ||
      result.receive.after.theme !== 'light' ||
      result.receive.after.layoutKey !== result.receive.before.layoutKey ||
      result.receive.after.avgPinned !== result.receive.before.avgPinned ||
      !same(result.receive.after.radio, result.receive.before.radio) ||
      (tier === 'phone' && result.receive.after.rxCollapsed !== 'false'))
      failures.push('receive-reset-touched-other-preferences');
    if (tier === 'desktop' && (!result.operations || result.operations.hidden.pinChecked !== false ||
      result.operations.hidden.drawerHidden !== 'true' || result.operations.hidden.display !== 'none' ||
      !result.operations.revealed.rowFound || !result.operations.revealed.menuClosed ||
      result.operations.revealed.drawerReveal !== 'true' || result.operations.revealed.drawerOpen !== 'true' ||
      result.operations.revealed.display === 'none' || !result.operations.revealed.targetVisible ||
      !result.operations.revealed.targetFocused || !result.operations.revealed.sameTarget ||
      !result.operations.rehidden.menuOpen || result.operations.rehidden.drawerReveal !== null ||
      result.operations.rehidden.display !== 'none' || result.operations.rehidden.drawerHidden !== 'true'))
      failures.push('hidden-operations-drawer-memory-route');
    const report = { scenario: scenario.name, result, failures, ok: failures.length === 0 };
    restoreReports.push(report);
    console.log(`${report.ok ? 'PASS' : 'FAIL'} ${scenario.name}: ${failures.join(', ') || 'cancel, global reset, receive-only reset, and radio safety'}`);
  }
  for (const scenario of pendingEntryScenarios) {
    await call('Emulation.setDeviceMetricsOverride', {
      width: scenario.width, height: scenario.height, deviceScaleFactor: 1, mobile: scenario.width < 600,
    });
    await call('Emulation.setTouchEmulationEnabled', { enabled: scenario.width < 600, maxTouchPoints: 1 });
    await call('Page.navigate', { url: pathToFileURL(scenario.file).href });
    let ready = false;
    for (let attempt = 0; attempt < 160; attempt++) {
      await pause(50);
      ready = await evaluate('document.readyState === "complete" && document.querySelectorAll("#settings-section-tabs .settings-section-tab").length === 11').catch(() => false);
      if (ready) break;
    }
    if (!ready) throw new Error(scenario.name + ': runtime did not initialize');
    const result = await evaluate(`(async () => {
      localStorage.setItem('saturn.remote.layout', 'desktop');
      window.applyLayout('desktop', false, false);
      const api = window.SaturnRemoteNextBundle.SaturnRemoteNext;
      const raf = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const visible = node => !!node && !node.closest('[hidden]') && node.getClientRects().length > 0;
      const open = () => {
        if (!document.getElementById('setup-menu').hidden) return;
        const header = document.getElementById('header-setup-btn');
        if (visible(header)) header.click();
        else {
          document.getElementById('phone-menu-btn').click();
          document.getElementById('phone-menu-settings-btn').click();
        }
      };
      const section = name => document.querySelector('.settings-section-tab[data-settings-section="' + name + '"]').click();
      const rowFor = id => [...document.querySelectorAll('#settings-section-page [data-settings-entry]')]
        .find(node => node.dataset.settingsEntry === id);
      const route = async (id, sectionId) => {
        open(); section(sectionId);
        const entry = api.SETTINGS_REGISTRY.find(item => item.id === id);
        const row = rowFor(id);
        const target = document.getElementById(entry.targetId);
        row?.click();
        await raf();
        return { id, targetId: entry.targetId, targetTag: target?.tagName,
          rowTag: row?.tagName, rowFound: !!row, targetCount: document.querySelectorAll('[id="' + entry.targetId + '"]').length,
          targetVisible: visible(target), targetFocused: document.activeElement === target || target?.contains(document.activeElement),
          menuOpen: !document.getElementById('setup-menu').hidden,
          indexActive: document.querySelector('.setup-stack')?.dataset.indexActive,
          mirror: rowFor(id)?.querySelector('.settings-entry-value')?.textContent?.trim() || '' };
      };
      const entries = {};
      entries.averaging = await route('display.averaging', 'display');
      const average = document.getElementById('display-spectrum-average');
      average.value = '4'; average.dispatchEvent(new Event('input', { bubbles: true })); updateUi();
      entries.averaging.action = { value: average.value, state: state.spectrumAverage,
        readout: document.getElementById('display-spectrum-average-readout').textContent.trim(),
        toolbar: document.getElementById('spectrum-average-toolbar-btn').textContent.trim() };
      entries.tunePeak = await route('display.tunePeak', 'display');
      const tune = document.getElementById('display-peak-tune-assist');
      tune.click();
      entries.tunePeak.action = { checked: tune.checked, state: state.peakTuneAssistEnabled,
        toolbarPressed: document.getElementById('spectrum-tune-peak-btn').getAttribute('aria-pressed') };
      entries.bandEdges = await route('display.bandEdges', 'display');
      const edges = document.getElementById('display-show-band-edges');
      edges.click();
      entries.bandEdges.action = { checked: edges.checked, state: state.showBandEdges,
        hiddenClass: document.getElementById('spectrum-shell').classList.contains('shell-no-band-edges') };
      entries.history = await route('display.history', 'display');
      const historyTarget = document.getElementById(entries.history.targetId);
      historyTarget.textContent = '12.5 s history';
      await new Promise(resolve => setTimeout(resolve, 300));
      entries.history.live = { source: historyTarget.textContent.trim(),
        mirror: rowFor('display.history')?.querySelector('.settings-entry-value')?.textContent?.trim() || '' };
      entries.analogRemoved = !window.SaturnRemoteNextBundle.SaturnRemoteNext.SETTINGS_REGISTRY
        .some(entry => entry.id === 'meter.analog') &&
        !document.getElementById('instrument-meter-mode').querySelector('option[value="analog"]') &&
        !visible(document.getElementById('meter-analog-option'));
      entries.dbfsBars = await route('meter.dbfsBars', 'meter');
      state.audioFramesPlayed = 1;
      state.rxAudioScopeLeftPeak = 0.8;
      state.rxAudioScopeRightPeak = 0.5;
      state.rxAudioScopeLeftPeakDbfs = -12.3;
      state.rxAudioScopeRightPeakDbfs = -24.6;
      updateRxAudioMeterElements();
      await new Promise(resolve => setTimeout(resolve, 300));
      entries.dbfsBars.live = { left: document.getElementById('instrument-rx-audio-left-peak').textContent.trim(),
        right: document.getElementById('instrument-rx-audio-right-peak').textContent.trim(),
        source: document.getElementById(entries.dbfsBars.targetId).textContent.trim(),
        mirror: rowFor('meter.dbfsBars')?.querySelector('.settings-entry-value')?.textContent?.trim() || '' };
      entries.band = await route('radio.band', 'radio');
      const bandOption = document.querySelector('#band-grid .band-btn[data-band="7150000"]');
      bandOption.click(); updateUi(); await raf();
      entries.band.action = { vfoA: state.vfoA, active: bandOption.classList.contains('active'),
        ariaChecked: bandOption.getAttribute('aria-checked'),
        vfoTag: document.getElementById('vfo-band-tag').textContent.trim() };
      entries.mode = await route('radio.mode', 'radio');
      const modeOption = document.querySelector('#mode-grid .mode-btn[data-mode="LSB"]');
      modeOption.click(); updateUi(); await raf();
      entries.mode.action = { mode: state.mode, active: modeOption.classList.contains('active'),
        ariaChecked: modeOption.getAttribute('aria-checked'),
        vfoTag: document.getElementById('vfo-mode-tag').textContent.trim() };
      open(); section('display');
      const pin = id => rowFor(id)?.closest('.settings-entry-shell')?.querySelector('.settings-entry-pin');
      const historyPin = pin('display.history');
      if (historyPin?.getAttribute('aria-pressed') === 'false') historyPin.click();
      section('meter');
      const dbfsPin = pin('meter.dbfsBars');
      if (dbfsPin?.getAttribute('aria-pressed') === 'false') dbfsPin.click();
      await raf();
      const trayRows = [...document.querySelectorAll('#main-screen-pins .main-screen-pin')];
      const tray = label => trayRows.find(node => node.textContent.includes(label));
      const historyRow = tray('Waterfall history');
      const dbfsRow = tray('Left and right dBFS bars');
      const tier = innerWidth < 600 ? 'phone' : 'desktop';
      const layout = api.loadLayout(tier);
      const pinned = { history: { stored: layout.pinned.includes('display.history'), row: !!historyRow,
          value: historyRow?.querySelector('.main-screen-pin-value')?.textContent?.trim() || '' },
        dbfsBars: { stored: layout.pinned.includes('meter.dbfsBars'), row: !!dbfsRow,
          left: dbfsRow?.querySelector('[data-meter-side="left"] .main-pin-meter-value')?.textContent?.trim() || '',
          right: dbfsRow?.querySelector('[data-meter-side="right"] .main-pin-meter-value')?.textContent?.trim() || '',
          leftFill: dbfsRow?.querySelector('[data-meter-side="left"] .main-pin-meter-fill')?.style.width || '',
          rightFill: dbfsRow?.querySelector('[data-meter-side="right"] .main-pin-meter-fill')?.style.width || '' } };
      return { entries, pinned };
    })()`);
    const png = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    writeFileSync(join(output, scenario.name + '.png'), Buffer.from(png.data, 'base64'));
    const failures = [];
    const entries = result.entries;
    for (const id of ['averaging', 'tunePeak', 'bandEdges']) {
      const row = entries[id];
      if (!row.rowFound || row.rowTag !== 'BUTTON' || row.targetCount !== 1 ||
        !row.targetVisible || !row.targetFocused || !row.menuOpen || row.indexActive !== 'false')
        failures.push(id + '-settings-route');
    }
    if (entries.averaging.action.state !== 4 || entries.averaging.action.readout !== '4 frames' ||
      !entries.averaging.action.toolbar.includes('4')) failures.push('averaging-action');
    if (entries.tunePeak.action.checked !== entries.tunePeak.action.state ||
      entries.tunePeak.action.toolbarPressed !== String(entries.tunePeak.action.state))
      failures.push('tune-peak-action');
    if (entries.bandEdges.action.checked !== entries.bandEdges.action.state ||
      entries.bandEdges.action.hiddenClass === entries.bandEdges.action.state)
      failures.push('band-edges-action');
    if (!entries.history.rowFound || entries.history.targetId !== 'terrain-status' ||
      entries.history.live.mirror !== entries.history.live.source) failures.push('history-live-readout');
    if (!entries.analogRemoved) failures.push('analog-meter-removal');
    if (!entries.dbfsBars.rowFound || entries.dbfsBars.targetId !== 'instrument-rx-audio-meters' ||
      !entries.dbfsBars.live.left.includes('-12.3 dBFS') ||
      !entries.dbfsBars.live.right.includes('-24.6 dBFS') ||
      !entries.dbfsBars.live.mirror.includes('-12.3 dBFS') ||
      !entries.dbfsBars.live.mirror.includes('-24.6 dBFS')) failures.push('dbfs-live-readout');
    for (const id of ['band', 'mode']) {
      const row = entries[id];
      if (!row.rowFound || row.rowTag !== 'BUTTON' || row.targetCount !== 1 ||
        !row.targetVisible || !row.targetFocused || row.menuOpen) failures.push(id + '-settings-route');
    }
    if (entries.band.action.vfoA !== 7150000 || !entries.band.action.active ||
      entries.band.action.ariaChecked !== 'true' || !entries.band.action.vfoTag.includes('40m'))
      failures.push('band-original-action');
    if (entries.mode.action.mode !== 'LSB' || !entries.mode.action.active ||
      entries.mode.action.ariaChecked !== 'true' || !entries.mode.action.vfoTag.includes('LSB'))
      failures.push('mode-original-action');
    if (!result.pinned.history.stored || !result.pinned.history.row ||
      !result.pinned.history.value.includes('history')) failures.push('history-main-pin');
    if (!result.pinned.dbfsBars.stored || !result.pinned.dbfsBars.row ||
      !result.pinned.dbfsBars.left.includes('-12.3 dBFS') ||
      !result.pinned.dbfsBars.right.includes('-24.6 dBFS') ||
      !result.pinned.dbfsBars.leftFill || !result.pinned.dbfsBars.rightFill)
      failures.push('dbfs-main-pin');
    const report = { scenario: scenario.name, result, failures, ok: failures.length === 0 };
    pendingEntryReports.push(report);
    console.log((report.ok ? 'PASS ' : 'FAIL ') + scenario.name + ': ' + (failures.join(', ') || 'seven entries, analog removal, and two live pins'));
  }
  for (const registryRouteScenario of registryRouteScenarios) {
  await call('Emulation.setDeviceMetricsOverride', {
    width: registryRouteScenario.width, height: registryRouteScenario.height, deviceScaleFactor: 1, mobile: false,
  });
  await call('Emulation.setTouchEmulationEnabled', { enabled: false, maxTouchPoints: 1 });
  await call('Page.navigate', { url: pathToFileURL(registryRouteScenario.file).href });
  let registryReady = false;
  for (let attempt = 0; attempt < 160; attempt++) {
    await pause(50);
    registryReady = await evaluate('document.readyState === "complete" && document.querySelectorAll("#settings-section-tabs .settings-section-tab").length === 11').catch(() => false);
    if (registryReady) break;
  }
  if (!registryReady) throw new Error('Registry route runtime did not initialize');
  await evaluate('localStorage.clear(); localStorage.setItem("saturn.remote.layout", "desktop")');
  await call('Page.reload', { ignoreCache: true });
  registryReady = false;
  for (let attempt = 0; attempt < 160; attempt++) {
    await pause(50);
    registryReady = await evaluate('document.readyState === "complete" && document.querySelectorAll("#settings-section-tabs .settings-section-tab").length === 11').catch(() => false);
    if (registryReady) break;
  }
  if (!registryReady) throw new Error('Fresh registry route runtime did not initialize');
  const registryResult = await evaluate(`(async () => {
    window.applyLayout('desktop', false, false);
    // A previous full-runtime probe may persist Lock even across file fixtures.
    // Use the original control to put this route audit in the unlocked state.
    if (state.frequencyLock) document.getElementById('freq-lock-btn').click();
    const api = window.SaturnRemoteNextBundle.SaturnRemoteNext;
    const entries = api.SETTINGS_REGISTRY.filter(entry => !!document.getElementById(entry.targetId));
    const results = [];
    const visible = node => !!node && !node.closest('[hidden]') && node.getClientRects().length > 0;
    const raf = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const open = () => {
      if (document.getElementById('setup-menu').hidden) document.getElementById('header-setup-btn').click();
    };
    document.querySelector('#radio-context-rail .context-tab[data-control-context="tx"]')?.click();
    open();
    document.querySelector('.settings-section-tab[data-settings-section="receive"]')?.click();
    const agcRoute = document.querySelector('#settings-section-page [data-settings-entry="receive.agc"]');
    agcRoute?.click();
    await raf();
    const receiveRouteRecovery = { row: !!agcRoute,
      selected: document.getElementById('radio-context-rail')?.dataset.activeContext,
      agcVisible: visible(document.getElementById('rx-agc-grid')) };
    const protectedAction = entry => entry.section === 'transmit'
      || /(?:ptt|mox|arm|disconnect|goLive|goOffline|delete|reset|wakeLock|freqLock)/i.test(entry.id + ' ' + entry.targetId);
    for (const entry of entries) {
      if (!document.getElementById('freq-entry-overlay').hidden)
        document.getElementById('freq-entry-cancel-btn').click();
      const terrainDialog = document.getElementById('terrain-settings');
      if (terrainDialog.open) {
        terrainDialog.querySelector('form[method="dialog"] button').click();
        await raf();
      }
      open();
      document.querySelector('.settings-section-tab[data-settings-section="' + entry.section + '"]')?.click();
      const row = [...document.querySelectorAll('#settings-section-page [data-settings-entry]')]
        .find(node => node.dataset.settingsEntry === entry.id);
      const target = document.getElementById(entry.targetId);
      const result = { id: entry.id, section: entry.section, kind: entry.targetKind,
        targetId: entry.targetId, row: !!row, rowTag: row?.tagName,
        targetCount: [...document.querySelectorAll('[id]')].filter(node => node.id === entry.targetId).length,
        route: '', reachable: false, eventForwarded: null, reason: null };
      if (!row || !target || result.targetCount !== 1) {
        result.reason = !row ? 'missing-row' : 'duplicate-target';
        results.push(result);
        continue;
      }
      const proxy = row.querySelector('.settings-entry-control');
      if (proxy) {
        result.route = 'native-proxy';
        result.reachable = visible(proxy);
        if (entry.section !== 'transmit' && !target.disabled && !proxy.disabled) {
          const seen = [];
          const record = event => seen.push(event.type);
          target.addEventListener('input', record, true);
          target.addEventListener('change', record, true);
          proxy.dispatchEvent(new Event('input', { bubbles: true }));
          proxy.dispatchEvent(new Event('change', { bubbles: true }));
          target.removeEventListener('input', record, true);
          target.removeEventListener('change', record, true);
          result.eventForwarded = seen.includes('input') && seen.includes('change');
          if (!result.eventForwarded) result.reason = 'native-proxy-no-original-event';
        } else {
          result.reason = 'disabled-or-transmit-proxy-skipped';
        }
      } else if (row.tagName !== 'BUTTON') {
        result.route = 'static-readout';
        const mirror = row.querySelector('.settings-entry-value');
        result.reachable = !!mirror;
        if (!mirror) result.reason = 'missing-readout-mirror';
      } else if (protectedAction(entry) ||
        (target.matches('button') && !target.closest('[data-setup-panel-id]'))) {
        result.route = 'protected-action';
        result.reachable = true;
        result.reason = 'action-not-activated';
      } else {
        result.route = 'navigation';
        if (entry.id === 'radio.keypad') result.beforeClick = {
          frequencyLock: state.frequencyLock,
          storedLock: localStorage.getItem('saturn.remote.freqLock'),
          overlayHidden: document.getElementById('freq-entry-overlay')?.hidden,
        };
        row.click();
        if (entry.id === 'radio.keypad') result.afterClick = {
          frequencyLock: state.frequencyLock,
          overlayHidden: document.getElementById('freq-entry-overlay')?.hidden,
          eventLogTail: document.getElementById('event-log')?.textContent?.slice(-350),
        };
        await raf();
        const panel = target.closest('[data-setup-panel-id]');
        const panelActive = !!panel && !panel.hidden && !document.getElementById('setup-menu').hidden;
        const focus = document.activeElement === target || target.contains(document.activeElement);
        const legacyPhoneDock = entry.id === 'shell.mobileDock' &&
          document.documentElement.dataset.layout === 'phone' && visible(target) && focus;
        result.reachable = (visible(target) && focus) || panelActive || legacyPhoneDock;
        result.targetVisible = visible(target);
        result.targetFocused = focus;
        result.panelActive = panelActive;
        if (!result.reachable) result.detail = {
          targetRectCount: target.getClientRects().length,
          targetDisplay: getComputedStyle(target).display,
          targetDisabled: target.disabled || false,
          terrainDialogOpen: document.getElementById('terrain-settings')?.open,
          frequencyOverlayHidden: document.getElementById('freq-entry-overlay')?.hidden,
          activeElement: document.activeElement?.id || document.activeElement?.tagName,
          runtimeError: window.saturnDiagLastError || null,
        };
        if (!result.reachable) result.reason = 'navigation-target-unreachable';
        if (entry.id === 'shell.mobileDock') {
          result.routedLayout = document.documentElement.dataset.layout;
          window.applyLayout('desktop', false, false);
          await raf();
        }
      }
      results.push(result);
    }
    return { totalRegistry: api.SETTINGS_REGISTRY.length, idBacked: entries.length,
      receiveRouteRecovery, results, layout: document.documentElement.dataset.layout };
  })()`);
  const registryGroups = {};
  if (!registryResult.receiveRouteRecovery.row || registryResult.receiveRouteRecovery.selected !== 'rx' ||
    !registryResult.receiveRouteRecovery.agcVisible) {
    registryGroups['receive-route-did-not-reveal-rx-tab'] = [registryResult.receiveRouteRecovery];
  }
  for (const item of registryResult.results) {
    if (!item.reason || ['action-not-activated', 'disabled-or-transmit-proxy-skipped'].includes(item.reason)) continue;
    (registryGroups[item.reason] ||= []).push(item.id);
  }
  const registryReport = { scenario: registryRouteScenario.name, ...registryResult,
    failuresByCause: registryGroups, ok: Object.keys(registryGroups).length === 0 };
  registryRouteReports.push(registryReport);
  writeFileSync(join(output, `${registryRouteScenario.name}.json`), JSON.stringify(registryReport, null, 2));
  console.log((registryReport.ok ? 'PASS ' : 'FAIL ') + registryRouteScenario.name +
    ': ' + registryResult.idBacked + ' ID-backed rows; ' +
    Object.entries(registryGroups).map(([cause, ids]) => cause + '=' + ids.length).join(', '));
  }
  writeFileSync(join(output, 'summary.json'), JSON.stringify({ generatedAt: new Date().toISOString(), reports, pttReports, settingsReports, presentationReports, disclosureReports, vfoReports, settingsIndexReports, exactEntryReports, systemRouteReports, mainScreenReports, restoreReports, pendingEntryReports, registryRouteReports }, null, 2));
} catch (error) {
  console.error(error instanceof Error ? error.stack : String(error));
  process.exitCode = 2;
} finally {
  clearTimeout(watchdog);
  if (chrome.exitCode === null) {
    await Promise.race([command('Browser.close').catch(() => {}), pause(1000)]);
  }
  chrome.kill();
}
const failures = reports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure.code} ${JSON.stringify(failure.detail)}`));
failures.push(...pttReports.flatMap(report => report.failures.map(failure => `PTT ${report.releaseType}: ${failure}`)));
failures.push(...settingsReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...presentationReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...disclosureReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...vfoReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...settingsIndexReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...exactEntryReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...systemRouteReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...mainScreenReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...restoreReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...pendingEntryReports.flatMap(report => report.failures.map(failure => `${report.scenario}: ${failure}`)));
failures.push(...registryRouteReports.flatMap(report => Object.entries(report.failuresByCause)
  .flatMap(([cause, ids]) => ids.map(id => `${report.scenario}: ${cause} ${id}`))));
console.log(`Output: ${output}`);
console.log(`Scenarios: ${reports.length}/${scenarios.length}; failed: ${reports.filter(report => !report.ok).length}; checks failed: ${failures.length}`);
console.log(`PTT resize cases: ${pttReports.filter(report => report.ok).length}/${pttReports.length} passed`);
console.log(`Settings cases: ${settingsReports.filter(report => report.ok).length}/${settingsReports.length} passed`);
console.log(`Presentation cases: ${presentationReports.filter(report => report.ok).length}/${presentationReports.length} passed`);
console.log(`Disclosure cases: ${disclosureReports.filter(report => report.ok).length}/${disclosureReports.length} passed`);
console.log(`VFO runtime cases: ${vfoReports.filter(report => report.ok).length}/${vfoReports.length} passed`);
console.log(`Settings index cases: ${settingsIndexReports.filter(report => report.ok).length}/${settingsIndexReports.length} passed`);
console.log(`RX exact-entry cases: ${exactEntryReports.filter(report => report.ok).length}/${exactEntryReports.length} passed`);
console.log(`System route cases: ${systemRouteReports.filter(report => report.ok).length}/${systemRouteReports.length} passed`);
console.log(`Main screen cases: ${mainScreenReports.filter(report => report.ok).length}/${mainScreenReports.length} passed`);
console.log(`Restore defaults cases: ${restoreReports.filter(report => report.ok).length}/${restoreReports.length} passed`);
console.log(`Pending entry cases: ${pendingEntryReports.filter(report => report.ok).length}/${pendingEntryReports.length} passed`);
console.log(`Registry route cases: ${registryRouteReports.filter(report => report.ok).length}/${registryRouteReports.length} passed`);
console.log(`Warnings: ${reports.reduce((sum, report) => sum + report.warnings.length, 0)}`);
if (failures.length) {
  for (const failure of failures) console.log(`  ${failure}`);
  process.exitCode ||= 1;
}
