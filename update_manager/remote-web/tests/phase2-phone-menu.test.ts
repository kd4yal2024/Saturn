import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

function section(startMarker: string, endMarker: string): string {
  const start = template.indexOf(startMarker);
  const end = template.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error(`Missing marker: ${startMarker}`);
  return template.slice(start, end);
}

const sheetMarkup = section('<!-- Phase 2: phone overflow menu.', '<script>\n      (function wirePhoneMenu()');
const script = section('(function wirePhoneMenu(){', 'window.saturnPhoneMenu =');

// Each route must delegate to a control that already exists in the template, so a
// rename cannot silently break the phone menu.
const ROUTE_TARGETS: Record<string, string[]> = {
  status: ['id="operator-conn-pill"'],
  system: ['class="header-diagnostics"', 'class="header-diagnostics-menu"'],
  help: ['id="shortcut-help-btn"'],
  phone: ['id="layout-btn"'],
  theme: ['id="theme-btn"'],
  settings: ['id="header-setup-btn"'],
  radioPath: ['data-phone-panel="routing"'],
  opsMemory: ['id="operations-tab-memory"'],
  opsAudio: ['id="operations-tab-audio"'],
  opsNetwork: ['id="operations-tab-network"'],
  opsDsp: ['id="operations-tab-dsp"'],
  opsRadio: ['id="operations-tab-radio"'],
  opsLog: ['id="operations-tab-log"'],
  opsRxMeasure: ['id="operations-tab-rx-measure"'],
};

describe('Phase 2 phone overflow menu', () => {
  it('exposes a labelled trigger wired to the sheet', () => {
    expect(template).toContain('id="phone-menu-btn"');
    expect(template).toContain('aria-controls="phone-menu-sheet"');
    expect(template).toContain('aria-haspopup="dialog"');
    expect(template).toContain('aria-expanded="false"');
    expect(template).toContain('aria-label="Open menu"');
  });

  it('declares the sheet as a hidden modal dialog with an accessible name', () => {
    expect(sheetMarkup).toContain('id="phone-menu-sheet"');
    expect(sheetMarkup).toContain('role="dialog"');
    expect(sheetMarkup).toContain('aria-modal="true"');
    expect(sheetMarkup).toContain('aria-label="Menu"');
    expect(sheetMarkup).toMatch(/id="phone-menu-sheet"[^>]*hidden/);
  });

  it('routes every menu entry to a control that still exists', () => {
    const declaredRoutes = Array.from(sheetMarkup.matchAll(/data-phone-route="([A-Za-z]+)"/g))
      .map((match) => String(match[1]));
    expect(declaredRoutes.sort()).toEqual(Object.keys(ROUTE_TARGETS).sort());
    for (const route of declaredRoutes) {
      expect(script).toContain(`${route}:`);
      for (const target of ROUTE_TARGETS[route] ?? []) {
        expect(template).toContain(target);
      }
    }
  });

  it('keeps focus management, escape handling and container-width switching', () => {
    expect(script).toContain("event.key === 'Escape'");
    expect(script).toContain("event.key !== 'Tab'");
    expect(script).toContain('preventScroll');
    expect(script).toContain("setAttribute('aria-expanded'");
    expect(script).toContain('ResizeObserver');
    expect(script).toContain('data-phone-menu');
    expect(script).toContain('PHONE_MENU_MAX_PX = 600');
    expect(script).toContain("document.querySelector('.app')");
  });

  it('keeps the System route reachable instead of opening a hidden details menu', () => {
    expect(sheetMarkup).toContain('id="phone-menu-system-host"');
    expect(sheetMarkup).toContain('id="phone-menu-groups"');
    expect(sheetMarkup).toContain('id="phone-menu-back-btn"');
    expect(script).toContain('function enterSystemView()');
    expect(script).toContain('function leaveSystemView()');
    expect(script).toContain('system: function(){ enterSystemView(); }');
    // The old behaviour opened a details element that phone mode hides.
    expect(script).not.toContain('details.open = true');
    // Leaving the sheet must put the hosted node back where it came from.
    expect(script).toContain('details.appendChild(systemSource)');
  });

  it('opens the existing Radio path panel when the phone menu route is chosen', () => {
    expect(template).toMatch(/<div class="panel" data-phone-panel="routing">/);
    expect(script).toContain("document.querySelector('[data-phone-panel=\"routing\"]')");
    expect(script).toContain("panel.dataset.phoneCollapsed === 'true'");
    expect(script).toContain("toggle.click()");
    expect(script).toContain('panel.scrollIntoView');
    expect(script).not.toContain('data-setup-panel="radio"');
  });

  it('uses vendored presentation assets and tokens for the trigger', () => {
    expect(sheetMarkup + template).toContain('sr-icon-button');
    expect(template).toContain('href="#sr-icon-menu"');
    expect(template).toContain('background: var(--overlay-backdrop)');
    expect(template).toContain('@media (pointer: coarse)');
    expect(template).not.toContain('background: rgba(0, 0, 0, 0.55)');
    expect(template).not.toContain('>&#9776;<');
  });

  it('neutralises the hosted header menu positioning inside the sheet', () => {
    // The original menu is position:absolute with a fixed width, which would escape
    // the sheet once hosted.
    const override = section('#phone-menu-system-host .header-diagnostics-menu {', '}');
    expect(override).toContain('position: static');
    expect(override).toContain('width: 100%');
    expect(override).toContain('right: auto');
  });

  it('does not duplicate the ids of the controls it routes to', () => {
    for (const id of ['phone-menu-btn', 'phone-menu-sheet', 'phone-menu-close-btn']) {
      const matches = template.match(new RegExp(`id="${id}"`, 'g')) || [];
      expect(matches.length).toBe(1);
    }
  });

  it('leaves TX markup and handlers untouched by the menu wiring', () => {
    expect(sheetMarkup).not.toMatch(/ptt|mox|arm/i);
    expect(script).not.toMatch(/setPtt|lockTx|armTxReady|toggleMox/);
  });
});
