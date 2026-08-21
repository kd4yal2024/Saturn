import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

const setupPanels = ['profiles', 'display', 'dsp', 'tx', 'network', 'audio', 'advanced'];

describe('responsive setup drawer template', () => {
  it('provides seven semantic setup tabs and panels', () => {
    for (const panel of setupPanels) {
      expect(template.match(new RegExp(`id="setup-tab-${panel}"`, 'g'))).toHaveLength(1);
      expect(template.match(new RegExp(`id="setup-panel-${panel}"`, 'g'))).toHaveLength(1);
      expect(template).toContain(`aria-controls="setup-panel-${panel}"`);
      expect(template).toContain(`data-setup-panel-id="${panel}" role="tabpanel"`);
    }
  });

  it('uses a modal drawer lifecycle with scrim, escape, and focus containment', () => {
    expect(template).toContain('id="setup-menu" role="dialog" aria-modal="true"');
    expect(template).toContain('id="setup-scrim" hidden');
    expect(template).toContain('if (event.key === "Escape")');
    expect(template).toContain('if (event.key !== "Tab") return');
    expect(template).toContain('document.body.classList.toggle("setup-open", next)');
  });

  it('places engineering routing only in Advanced', () => {
    const advancedStart = template.indexOf('id="setup-panel-advanced"');
    const setupEnd = template.indexOf('class="top-meter-bank"', advancedStart);
    expect(template.indexOf('id="ws-url"')).toBeGreaterThan(advancedStart);
    expect(template.indexOf('id="ws-url"')).toBeLessThan(setupEnd);
    expect(template.indexOf('id="sample-rate"')).toBeGreaterThan(advancedStart);
    expect(template.indexOf('id="sample-rate"')).toBeLessThan(setupEnd);
    expect(template.match(/id="ws-url"/g)).toHaveLength(1);
    expect(template.match(/id="sample-rate"/g)).toHaveLength(1);
  });

  it('separates profile deletion from routine profile actions', () => {
    const deleteButton = template.indexOf('id="setup-profile-delete-btn"');
    const dangerZone = template.lastIndexOf('class="setup-danger-zone"', deleteButton);
    expect(dangerZone).toBeGreaterThanOrEqual(0);
    expect(deleteButton).toBeGreaterThan(dangerZone);
  });
});
