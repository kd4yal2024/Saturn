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

for (const scenario of scenarios) {
  scenario.file = join(output, `${scenario.name}.html`);
  writeFileSync(scenario.file, pageFor(scenario));
}

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
  const responsiveConsole = ${JSON.stringify(scenario.layout !== 'phone')};
  window.__uiFixtureApplyControlContext = context => {
    rail.dataset.activeContext = context;
    rail.querySelectorAll('.context-tab').forEach(button => {
      const active = button.dataset.controlContext === context;
      button.setAttribute('aria-selected', active ? 'true' : 'false');
      button.tabIndex = active ? 0 : -1;
    });
    audioStrip.hidden = !responsiveConsole && context === 'tx';
    audioStrip.querySelectorAll('[data-control-context]').forEach(control => {
      control.hidden = responsiveConsole
        ? control.dataset.controlContext === 'dsp' && context !== 'dsp'
        : control.dataset.controlContext !== context;
    });
    rightRail.dataset.contextCompact = responsiveConsole || context === 'tx' ? 'false' : 'true';
  };
  window.__uiFixtureApplyControlContext('rx');
  return { railParent: rail.parentElement?.className, audioParent: audioStrip.parentElement?.id,
    txParent: rightRail.parentElement?.id };
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
  const txBadge = firstVisible(['#tx-status-badge', '#tx-state-badge', '.tx-badge', '[data-ui="tx-badge"]', '#operator-rxtx-pill', '#tx-zone-state']);
  const display = firstVisible(['#display-well', '.display-stack', '#spectrum-shell', '[data-ui="display-well"]']);
  const spectrum = firstVisible(['#terrain-canvas', '#spectrum-shell']);
  const waterfall = firstVisible(['#waterfall-shell']);
  const dock = document.getElementById('mobile-control-dock');
  const txHost = document.querySelector('#tx-safety-host.phone-tx-bar');
  const txArm = document.getElementById('tx-arm-btn');
  const ptt = document.getElementById('ptt-btn');
  const stickyTx = firstVisible(['#tx-safety-host.phone-tx-bar', '#tx-sticky-bar', '#tx-safety-bar', '#mobile-tx-bar', '.tx-sticky-bar', '.tx-safety-bar', '.mobile-tx-bar', '[data-tx-sticky]']);
  const audioStrip = document.querySelector('.audio-control-strip');
  const txSurfaceSelectors = ['#tx-safety-host.phone-tx-bar', '.right-rail [data-phone-panel="tx"]', '#tx-zone'];
  const txSurface = firstVisible(txSurfaceSelectors);
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
  let simultaneousRxTx = null;
  let armedPttSpan = null;
  let overlayHits = null;
  if (scenario.layout !== 'phone') {
    check(!!rail && audioStrip?.parentElement === rail && document.querySelector('.right-rail')?.parentElement === rail,
      'context-rail-reparent-missing', { audioParent: audioStrip?.parentElement?.id, txParent: document.querySelector('.right-rail')?.parentElement?.id });
    const rxVisible = visible(audioStrip) && visible(txSurface);
    window.__uiFixtureApplyControlContext('tx');
    const txSurfaceAfter = firstVisible(txSurfaceSelectors);
    const txVisible = visible(audioStrip) && visible(txSurfaceAfter);
    const txContext = rail?.dataset.activeContext;
    window.__uiFixtureApplyControlContext('rx');
    simultaneousRxTx = { rxVisible, txVisible, txContext, audioSelector: selector(audioStrip),
      txSelectorRx: selector(txSurface), txSelectorTx: selector(txSurfaceAfter), contextRail: selector(rail) };
    check(rxVisible && txVisible && txContext === 'tx', 'rx-tx-not-simultaneous',
      { ...simultaneousRxTx, audio: rect(audioStrip), tx: rect(txSurfaceAfter) });
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
    duplicateIds, simultaneousRxTx, armedPttSpan, overlayHits,
    controls: { visible: visibleControls.length, offscreen: offscreenControls },
    hitTests: { txHost: hitTest(txHost), txArm: hitTest(txArm), ptt: hitTest(ptt), dock: hitTest(dock) },
    regions: Object.fromEntries(Object.entries({ app, bar, left, center, right, vfo, txBadge, display, spectrum, waterfall, dock, txHost, stickyTx })
      .map(([name, e]) => [name, { selector: selector(e), rect: rect(e) }])),
    failures, warnings, ok: failures.length === 0,
  };
})()`;

const reports = [];
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
console.log(`Output: ${output}`);
console.log(`Scenarios: ${reports.length}/${scenarios.length}; failed: ${reports.filter(report => !report.ok).length}; checks failed: ${failures.length}`);
console.log(`Warnings: ${reports.reduce((sum, report) => sum + report.warnings.length, 0)}`);
if (failures.length) {
  for (const failure of failures) console.log(`  ${failure}`);
  process.exitCode ||= 1;
}
