import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { SETTINGS_REGISTRY } from '../src/settings/registry';
import { WORKBENCH_PAGES, WORKBENCH_EXTRAS, workbenchEntries, workbenchPage, workbenchGroup, settingScope } from '../src/settings/workbench-catalog';
const template = readFileSync(new URL('../../templates/saturn-remote-next.html', import.meta.url), 'utf8');
const all = [...SETTINGS_REGISTRY, ...WORKBENCH_EXTRAS];
const entry = (id: string) => { const item = all.find(entry => entry.id === id); if (!item) throw Error(id); return item; };
describe('Settings workbench catalog', () => {
  it('places every existing indexed setting in a named category and group', () => {
    for (const item of all) {
      expect(WORKBENCH_PAGES.some(page => page.id === workbenchPage(item)), item.id).toBe(true);
      expect(workbenchGroup(item), item.id).not.toBe('');
    }
    expect(new Set(all.map(entry => entry.id)).size).toBe(all.length);
  });
  it('gives PureSignal its own page without moving unrelated TX controls', () => {
    for (const id of ['transmit.psAdaptation','transmit.puresignal','transmit.psFeedback','transmit.psDuration']) expect(workbenchPage(entry(id))).toBe('ps');
    expect(workbenchPage(entry('transmit.drive'))).toBe('transmit');
    expect(workbenchPage(entry('memory.saveCurrent'))).toBe('radio');
  });
  it('groups detailed processing separately from keying/status and launch overrides', () => {
    expect(workbenchGroup(entry('transmit.arm'))).toBe('Operating status');
    expect(workbenchGroup(entry('transmit.noiseGateThreshold'))).toBe('Voice processing');
    expect(workbenchGroup(entry('receive.nr2GainMethod'))).toBe('Noise reduction');
    expect(workbenchGroup(entry('transmit.twoToneFreq1'))).toBe('Two-tone generator');
    expect(workbenchGroup(entry('transmit.psResult'))).toBe('Host reports');
    expect(workbenchGroup(entry('about.txLevelOverride'))).toBe('Launch options');
  });
  it('labels existing per-band controls and global devices honestly', () => {
    for (const id of ['transmit.micGain','transmit.filterLow','receive.headphones','radio.antenna','transmit.puresignalAttenuation']) expect(settingScope(entry(id))).toBe('Per band');
    expect(settingScope(entry('audio.inputSelect'))).toBe('Browser / session');
    expect(settingScope(entry('transmit.psFeedback'))).toBe('Live status');
    expect(settingScope(entry('transmit.ptt'))).toBe('Control');
  });
  it('all supplemental controls resolve to real, unique shipped template targets', () => {
    for (const item of WORKBENCH_EXTRAS) expect(template.split(`id="${item.targetId}"`).length - 1, item.id).toBe(1);
  });
  it('omits optional controls from hosts/views that do not supply them', () => {
    const doc = { getElementById: () => null } as unknown as Document;
    expect(workbenchEntries(doc)).toEqual(SETTINGS_REGISTRY);
  });
  it('keeps the production launch in the existing session and retains the legacy layout editor', () => {
    expect(template).toContain('$("header-setup-btn")?.addEventListener("click", openSettingsWorkbench);');
    expect(template).toContain('new _next.SettingsWorkbench({');
    expect(template).toContain('reveal: (entry) => revealSettingsEntry(entry)');
    expect(template).toContain('setSettingsSection("interface")');
  });
});
