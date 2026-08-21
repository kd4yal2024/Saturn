import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

const targets = ['memory', 'audio', 'network', 'dsp', 'radio', 'log'];

describe('responsive operations drawer template', () => {
  it('provides one tab and one panel for every operations view', () => {
    for (const target of targets) {
      expect(template.match(new RegExp(`id="operations-tab-${target}"`, 'g'))).toHaveLength(1);
      expect(template.match(new RegExp(`id="operations-panel-${target}"`, 'g'))).toHaveLength(1);
      expect(template).toContain(`aria-controls="operations-panel-${target}"`);
      expect(template).toContain(`data-operations-panel="${target}"`);
    }
  });

  it('moves the existing operator log into its drawer panel', () => {
    expect(template).toContain('const logPanel = $("operations-panel-log")');
    expect(template).toContain('logPanel.appendChild(operatorLog)');
  });

  it('persists drawer selection separately from radio profiles', () => {
    expect(template).toContain('const OPERATIONS_DRAWER_PREF_KEY = "saturn.remote.operationsDrawer"');
    const start = template.indexOf('function applyOperationsDrawerSelection(');
    const end = template.indexOf('function initResponsiveShell()', start);
    const handler = template.slice(start, end);
    expect(handler).toContain('localStorage.setItem(OPERATIONS_DRAWER_PREF_KEY');
    expect(handler).not.toContain('scheduleRemoteSettingsSave()');
  });

  it('renders each telemetry view only while that drawer panel is open', () => {
    expect(template).toContain('if (!selection.open || selection.target === "log") return');
    expect(template).toContain('now - operationsDrawerLastRenderAt < 500');
    expect(template).toContain('updateOperationsDrawer();');
  });
});
