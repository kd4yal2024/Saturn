import css from '../../styles/settings-workbench.css?inline';
import { panelGeometry } from '../puresignal/panel-layout';
import { type SettingsEntry } from './registry';
import { WORKBENCH_PAGES, workbenchEntries, workbenchGroup, workbenchPage, settingScope, type WorkbenchPage } from './workbench-catalog';

export interface SettingsWorkbenchHost {
  document: Document;
  status(): { connected: boolean; band: string; frequency: number; mode: string; profile: string; tx: string };
  reveal(entry: SettingsEntry): void;
  layout(): void;
}

const layoutKey = 'saturn.settings.window.v1';
const sectionKey = 'saturn.settings.workbench.page.v1';
const noClickProxy = new Set(['transmit.arm', 'transmit.ptt', 'transmit.mox', 'shell.settings']);
const isField = (node: HTMLElement): node is HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement =>
  /^(INPUT|SELECT|TEXTAREA)$/.test(node.tagName);
const text = (node: Element | null | undefined) => node?.textContent?.trim() || '';

/** A second view of the existing controls, never another radio/transport owner.
 * Events are created in the canonical control's realm (not the popup's realm).
 * No credentials, audio capture, radio commands or state copies live here.
 */
export class SettingsWorkbench {
  private popup: Window | null = null;
  private root: HTMLElement | null = null;
  private overlay: HTMLElement | null = null;
  private timer: number | undefined;
  private timerWindow: Window | null = null;
  private refreshers: (() => void)[] = [];
  private cleanups: (() => void)[] = [];
  private page: WorkbenchPage = 'radio';
  private group = '';
  private query = '';
  private entries: SettingsEntry[] = [];
  private trigger: HTMLElement | null = null;
  private statusLine: HTMLElement | null = null;
  private resultLine: HTMLElement | null = null;
  private content: HTMLElement | null = null;
  private nav: HTMLElement | null = null;
  private tabs: HTMLElement | null = null;
  private search: HTMLInputElement | null = null;
  private contextKey = '';

  constructor(private host: SettingsWorkbenchHost) {
    host.document.defaultView?.addEventListener('pagehide', () => this.close(false));
  }

  open(): void {
    if (this.root?.isConnected && (!this.popup || !this.popup.closed)) {
      this.popup?.focus(); this.search?.focus(); return;
    }
    this.close(false);
    const main = this.host.document.defaultView!;
    this.trigger = this.host.document.activeElement as HTMLElement | null;
    let saved: unknown;
    try { saved = JSON.parse(main.localStorage.getItem(layoutKey) || 'null'); } catch { /* Optional layout storage. */ }
    if (!main.matchMedia('(max-width: 767px)').matches) {
      const screen = main.screen as Screen & { availLeft?: number; availTop?: number };
      const g = panelGeometry(saved, { width: screen.availWidth, height: screen.availHeight, left: screen.availLeft || 0, top: screen.availTop || 0 }, { width: 1240, height: 850 });
      this.popup = main.open('', '_blank', `popup,width=${g.width},height=${g.height},left=${g.left},top=${g.top}`);
    }
    const doc = this.popup?.document || this.host.document;
    if (this.popup) {
      this.popup.opener = null;
      doc.title = 'Saturn Remote — Settings';
      doc.documentElement.lang = 'en';
      doc.body.style.cssText = 'margin:0;overflow:hidden;';
    }
    const style = doc.createElement('style');
    style.textContent = css; doc.head.append(style);
    this.cleanups.push(() => style.remove());
    const root = doc.createElement('section'); this.root = root;
    root.id = 'saturn-settings-workbench'; root.className = 'sw-root';
    root.setAttribute('aria-label', 'Saturn Remote settings');
    if (!this.popup) {
      const overlay = doc.createElement('div'); overlay.className = 'sw-overlay';
      this.overlay = overlay; overlay.append(root); doc.body.append(overlay);
      root.setAttribute('role', 'dialog'); root.setAttribute('aria-modal', 'true');
      overlay.addEventListener('click', event => { if (event.target === overlay) this.close(); });
    } else doc.body.append(root);
    this.entries = workbenchEntries(this.host.document);
    try {
      const page = main.localStorage.getItem(sectionKey);
      if (WORKBENCH_PAGES.some(item => item.id === page)) this.page = page as WorkbenchPage;
    } catch { /* Optional preference. */ }
    this.query = ''; this.group = '';
    this.mount(doc);
    this.timerWindow = this.popup || main;
    this.timer = this.timerWindow.setInterval(() => {
      if (this.popup?.closed) { this.close(); return; }
      this.sync();
    }, 250);
    if (this.popup) {
      const child = this.popup;
      const save = () => {
        if (child.closed) return;
        try { main.localStorage.setItem(layoutKey, JSON.stringify({ width: child.innerWidth, height: child.innerHeight, left: child.screenX, top: child.screenY })); } catch { /* Optional layout. */ }
      };
      child.addEventListener('resize', save);
      child.addEventListener('pagehide', save);
      const closed = () => this.close();
      child.addEventListener('pagehide', closed);
      this.cleanups.push(() => { child.removeEventListener('resize', save); child.removeEventListener('pagehide', save); child.removeEventListener('pagehide', closed); save(); });
    }
    this.sync(); this.search?.focus();
  }

  close(focus = true): void {
    if (this.timer !== undefined) this.timerWindow?.clearInterval(this.timer);
    this.timer = undefined; this.timerWindow = null;
    this.cleanups.splice(0).forEach(cleanup => cleanup());
    const popup = this.popup; this.popup = null;
    this.overlay?.remove(); this.overlay = null;
    this.root?.remove(); this.root = null;
    this.refreshers = [];
    this.contextKey = '';
    if (popup && !popup.closed) popup.close();
    if (focus && this.trigger?.isConnected) this.trigger.focus();
    this.trigger = null;
  }

  private node<K extends keyof HTMLElementTagNameMap>(doc: Document, tag: K, className = '', label = ''): HTMLElementTagNameMap[K] {
    const node = doc.createElement(tag); node.className = className; node.textContent = label; return node;
  }

  private button(doc: Document, label: string, action: () => void, className = ''): HTMLButtonElement {
    const button = this.node(doc, 'button', className, label); button.type = 'button';
    button.addEventListener('click', () => { if (button.isConnected) action(); }); return button;
  }

  private onMain(action: () => void): void {
    const main = this.host.document.defaultView;
    // Do not leave the phone fallback covering the control we just revealed.
    if (!this.popup) this.close(false);
    main?.focus(); action();
  }

  private mount(doc: Document): void {
    const root = this.root!;
    const header = this.node(doc, 'header', 'sw-header');
    const brand = this.node(doc, 'div', 'sw-brand');
    brand.append(this.node(doc, 'span', 'sw-eyebrow', 'SATURN REMOTE'), this.node(doc, 'h1', '', 'Settings'));
    const searchWrap = this.node(doc, 'div', 'sw-search-wrap');
    this.search = this.node(doc, 'input', 'sw-search'); this.search.type = 'search';
    this.search.placeholder = 'Search all settings… e.g. mic, NR2, two-tone';
    this.search.setAttribute('aria-label', 'Search all settings');
    this.search.addEventListener('input', () => { this.query = this.search!.value; this.render(); });
    searchWrap.append(this.search);
    header.append(brand, searchWrap, this.button(doc, 'Close', () => this.close(), 'sw-close'));
    this.statusLine = this.node(doc, 'div', 'sw-context');
    const body = this.node(doc, 'div', 'sw-body');
    this.nav = this.node(doc, 'nav', 'sw-nav'); this.nav.setAttribute('aria-label', 'Settings categories');
    WORKBENCH_PAGES.forEach(page => {
      const button = this.button(doc, '', () => {
        this.page = page.id; this.group = ''; this.query = ''; this.search!.value = '';
        try { this.host.document.defaultView?.localStorage.setItem(sectionKey, page.id); } catch { /* Optional. */ }
        this.render();
      }, 'sw-nav-button');
      button.dataset.page = page.id;
      const icon = this.node(doc, 'span', 'sw-nav-icon', page.icon); icon.setAttribute('aria-hidden', 'true');
      button.append(icon, this.node(doc, 'span', '', page.label));
      this.nav!.append(button);
    });
    const main = this.node(doc, 'main', 'sw-main');
    this.tabs = this.node(doc, 'nav', 'sw-tabs'); this.tabs.setAttribute('aria-label', 'Settings subcategories');
    this.content = this.node(doc, 'div', 'sw-content'); this.content.id = 'settings-workbench-content';
    main.append(this.tabs, this.content); body.append(this.nav, main);
    const footer = this.node(doc, 'footer', 'sw-footer');
    const note = this.node(doc, 'div');
    note.append(this.node(doc, 'strong', '', 'Changes apply immediately'), this.node(doc, 'span', '', ' · Per-band controls follow the active band. Devices and connection preferences stay global.'));
    this.resultLine = this.node(doc, 'div', 'sw-result', 'Uses the open radio session. Keep Saturn Remote open.');
    this.resultLine.setAttribute('role', 'status'); this.resultLine.setAttribute('aria-live', 'polite');
    const tools = this.node(doc, 'div', 'sw-footer-tools');
    tools.append(this.button(doc, 'Main-screen layout & pins ↗', () => this.onMain(() => this.host.layout())));
    footer.append(note, this.resultLine, tools);
    root.append(header, this.statusLine, body, footer);
    root.addEventListener('keydown', event => {
      // The embedded fallback must not pass Space/tuning shortcuts through to
      // the radio beneath it. Key-up still propagates so pending PTT can release.
      event.stopPropagation();
      if (event.key === 'Escape') { event.preventDefault(); this.close(); return; }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); this.search?.focus(); }
      if (!this.popup && event.key === 'Tab') {
        const nodes = Array.from(root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]')).filter(node => node.getClientRects().length);
        const index = nodes.indexOf(doc.activeElement as HTMLElement);
        if (event.shiftKey && index <= 0) { event.preventDefault(); nodes.at(-1)?.focus(); }
        else if (!event.shiftKey && index === nodes.length - 1) { event.preventDefault(); nodes[0]?.focus(); }
      }
    });
    this.render();
  }

  private render(): void {
    const doc = this.root!.ownerDocument, page = WORKBENCH_PAGES.find(page => page.id === this.page)!;
    this.refreshers = [];
    this.content!.replaceChildren(); this.tabs!.replaceChildren();
    this.nav!.querySelectorAll<HTMLElement>('[data-page]').forEach(button => {
      const active = !this.query.trim() && button.dataset.page === this.page;
      button.setAttribute('aria-current', active ? 'page' : 'false');
    });
    const query = this.query.trim().toLowerCase();
    const entries = query ? this.entries.filter(entry =>
      [entry.label, entry.description, entry.id, entry.targetId, ...entry.synonyms].some(value => value.toLowerCase().includes(query)))
      : this.entries.filter(entry => workbenchPage(entry) === this.page);
    const heading = this.node(doc, 'div', 'sw-page-heading');
    heading.append(this.node(doc, 'p', 'sw-eyebrow', query ? 'SEARCH ALL SETTINGS' : 'SETUP / ' + page.label.toUpperCase()),
      this.node(doc, 'h2', '', query ? `Results for “${this.query.trim()}”` : page.label),
      this.node(doc, 'p', 'sw-description', query ? `${entries.length} settings found. Edit directly below or open the related control.` : page.detail));
    this.content!.append(heading);
    const order = ['Tuning & signal paths', 'Bands & memories', 'Profiles', 'Audio & gain', 'Passband & equalizer', 'Noise reduction', 'Notch & noise blanker', 'Source, level & passband', 'Equalizer & CFC', 'Voice processing', 'Two-tone generator', 'Monitor & timing', 'Operating status', 'Adaptation & calibration', 'Feedback path', 'Measurement tools', 'Host reports', 'Spectrum', 'Waterfall', '3D terrain', 'Saved preferences', 'Launch options'];
    const groups = [...new Set(entries.map(workbenchGroup))].sort((a, b) => {
      const rank = (value: string) => order.includes(value) ? order.indexOf(value) : 0;
      return rank(a) - rank(b);
    });
    if (!groups.includes(this.group)) this.group = '';
    if (!query) {
      for (const group of ['', ...groups]) {
        const button = this.button(doc, group || 'Overview', () => { this.group = group; this.render(); }, 'sw-tab');
        button.setAttribute('aria-current', group === this.group ? 'page' : 'false'); this.tabs!.append(button);
      }
    }
    const grid = this.node(doc, 'div', 'sw-groups');
    for (const group of groups) {
      if (!query && this.group && this.group !== group) continue;
      const collapsed = !query && !this.group && ['Launch options', 'Saved preferences'].includes(group);
      const card = this.node(doc, collapsed ? 'details' : 'section', 'sw-card');
      card.append(this.node(doc, collapsed ? 'summary' : 'h3', '', group));
      const list = this.node(doc, 'div', 'sw-fields');
      for (const entry of entries.filter(entry => workbenchGroup(entry) === group)) list.append(this.row(doc, entry));
      card.append(list); grid.append(card);
    }
    this.content!.append(grid);
    if (!entries.length) this.content!.append(this.node(doc, 'p', 'sw-empty', 'No matching setting. Try a shorter name such as “EQ”, “MON”, “ATT” or “SATP”.'));
    this.sync(); this.content!.scrollTo?.({ top: 0 });
  }

  private row(doc: Document, entry: SettingsEntry): HTMLElement {
    const row = this.node(doc, 'div', 'sw-row'); row.dataset.setting = entry.id;
    const copy = this.node(doc, 'div', 'sw-copy');
    const label = this.node(doc, 'label', 'sw-label', entry.label);
    const scope = this.node(doc, 'span', 'sw-scope', settingScope(entry));
    const description = this.node(doc, 'p', 'sw-help', entry.description);
    description.id = `sw-help-${entry.id.replace(/[^a-zA-Z0-9-]/g, '-')}`;
    copy.append(label, scope, description); row.append(copy);
    const control = this.node(doc, 'div', 'sw-control'); row.append(control);
    const target = this.host.document.getElementById(entry.targetId);
    if (target && isField(target) && !['button', 'submit', 'hidden', 'file', 'password'].includes(target.getAttribute('type') || '')) {
      this.field(doc, target, control, label, description, entry);
    } else if (target?.matches('button') && !noClickProxy.has(entry.id) && !/operationsTab|dspTab|puresignalOpen|shell\./.test(entry.id) && !/copy/i.test(target.id)) {
      const proxy = this.button(doc, '', () => {
        if (!target.isConnected || target.matches(':disabled')) return;
        // Only an explicit user click reaches the existing authoritative action.
        target.click(); this.resultLine!.textContent = `${entry.label}: request sent through the radio controls.`; this.sync();
      }, 'sw-action');
      proxy.setAttribute('aria-label', entry.label); control.append(proxy);
      this.refreshers.push(() => {
        proxy.disabled = !target.isConnected || target.matches(':disabled');
        proxy.textContent = text(target) || entry.label;
        const pressed = target.getAttribute('aria-pressed');
        if (pressed !== null) proxy.setAttribute('aria-pressed', pressed); else proxy.removeAttribute('aria-pressed');
        proxy.title = target.title;
      });
    } else if (target && ['readout', 'indicator'].includes(entry.targetKind)) {
      const value = this.node(doc, 'output', 'sw-value'); control.append(value);
      this.refreshers.push(() => { value.textContent = text(target) || target.getAttribute('aria-label') || 'Waiting for report'; });
    } else if (target && ['radio.band', 'radio.mode', 'receive.agc'].includes(entry.id)) {
      control.classList.add('sw-choices');
      target.querySelectorAll<HTMLButtonElement>('button').forEach(original => {
        const proxy = this.button(doc, text(original), () => { if (!original.disabled && original.isConnected) { original.click(); this.sync(); } });
        control.append(proxy);
        this.refreshers.push(() => { proxy.disabled = original.disabled || !original.isConnected; proxy.setAttribute('aria-pressed', original.getAttribute('aria-checked') === 'true' || original.classList.contains('active') ? 'true' : 'false'); });
      });
    } else if (entry.targetKind === 'query-param' || entry.targetKind === 'storage-key') {
      const output = this.node(doc, 'output', 'sw-value'); control.append(output);
      this.refreshers.push(() => {
        try {
          const main = this.host.document.defaultView!;
          const value = entry.targetKind === 'query-param' ? new URL(main.location.href).searchParams.get(entry.targetId) : main.localStorage.getItem(entry.targetId);
          output.textContent = value === null ? 'Default' : value.length > 100 ? 'Saved in this browser' : value;
          output.title = entry.targetId;
        } catch { output.textContent = 'Storage unavailable'; }
      });
    } else if (target) {
      control.append(this.button(doc, noClickProxy.has(entry.id) ? 'On main TX panel ↗' : 'Open on main screen ↗', () => this.onMain(() => this.host.reveal(entry))));
    } else {
      control.append(this.node(doc, 'span', 'sw-unavailable', 'Not available in this view'));
    }
    return row;
  }

  private field(doc: Document, target: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, holder: HTMLElement, label: HTMLLabelElement, description: HTMLElement, entry: SettingsEntry): void {
    const proxy = doc.createElement(target.tagName.toLowerCase()) as typeof target;
    proxy.className = 'sw-input'; proxy.id = `sw-input-${entry.id.replace(/[^a-zA-Z0-9-]/g, '-')}`;
    label.htmlFor = proxy.id; proxy.setAttribute('aria-describedby', description.id);
    if (proxy.tagName === 'INPUT') (proxy as HTMLInputElement).type = target.getAttribute('type') || 'text';
    holder.append(proxy);
    let exact: HTMLInputElement | null = null;
    if (target.getAttribute('type') === 'range') {
      holder.classList.add('sw-range'); exact = this.node(doc, 'input', 'sw-exact'); exact.type = 'number';
      exact.setAttribute('aria-label', `${entry.label}, exact value`); exact.setAttribute('aria-describedby', description.id); holder.append(exact);
    }
    const value = this.node(doc, 'output', 'sw-current'); holder.append(value);
    const relay = (event: Event) => {
      const source = event.currentTarget as HTMLInputElement;
      if (!source.isConnected || !proxy.isConnected || !target.isConnected || target.matches(':disabled')) { sync(); return; }
      if (!source.checkValidity()) { source.reportValidity(); return; }
      if (target.getAttribute('type') === 'checkbox') (target as HTMLInputElement).checked = source.checked;
      else target.value = source.value;
      const EventClass = this.host.document.defaultView!.Event;
      // Exact-number edits commit both phases; normal controls keep their own
      // input/change cadence (including existing per-band saves and TX batching).
      if (source === exact && event.type === 'change') target.dispatchEvent(new EventClass('input', { bubbles: true }));
      target.dispatchEvent(new EventClass(event.type, { bubbles: true }));
      this.resultLine!.textContent = `${entry.label}: ${settingScope(entry) === 'Per band' ? 'updated for the active band' : 'updated'}.`;
      sync();
    };
    proxy.addEventListener('input', relay); proxy.addEventListener('change', relay);
    exact?.addEventListener('change', relay);
    const sync = () => {
      const disabled = !target.isConnected || target.matches(':disabled'); proxy.disabled = disabled; if (exact) exact.disabled = disabled;
      if (target.tagName === 'SELECT') {
        const original = target as HTMLSelectElement, select = proxy as HTMLSelectElement;
        if (select.innerHTML !== original.innerHTML) select.innerHTML = original.innerHTML;
      }
      for (const attr of ['min', 'max', 'step', 'maxlength', 'placeholder', 'pattern', 'required', 'readonly']) {
        const attribute = target.getAttribute(attr);
        for (const copy of [proxy, exact]) if (copy) { if (attribute === null) copy.removeAttribute(attr); else copy.setAttribute(attr, attribute); }
      }
      if (target.getAttribute('type') === 'checkbox') (proxy as HTMLInputElement).checked = (target as HTMLInputElement).checked;
      else if (doc.activeElement !== proxy) proxy.value = target.value;
      if (exact && doc.activeElement !== exact) exact.value = target.value;
      const originalReadout = this.host.document.getElementById(target.id + '-readout')
        || target.closest('.eq-band')?.querySelector('.eq-val');
      value.textContent = text(originalReadout) || (target.tagName === 'SELECT' ? (target as HTMLSelectElement).selectedOptions[0]?.textContent || '' : target.getAttribute('type') === 'checkbox' ? (target as HTMLInputElement).checked ? 'On' : 'Off' : target.value);
      value.title = disabled ? target.title || 'The radio currently marks this control unavailable.' : '';
      holder.dataset.available = String(!disabled);
    };
    this.refreshers.push(sync);
  }

  private sync(): void {
    if (!this.root?.isConnected) return;
    const state = this.host.status();
    const key = JSON.stringify([state.band, state.mode, state.profile]);
    const changed = this.contextKey && this.contextKey !== key;
    this.contextKey = key;
    if (changed) {
      // A half-edited value from the old band/profile must not commit into the
      // newly selected context. Rebuild controls; detached relays reject events.
      this.render();
      this.resultLine!.textContent = 'Band, mode or profile changed. Controls refreshed for the current settings.';
      return;
    }
    this.root.dataset.theme = this.host.document.documentElement.dataset.theme || 'dark';
    this.statusLine!.replaceChildren();
    const doc = this.root.ownerDocument;
    const connection = this.node(doc, 'span', 'sw-connection', state.connected ? '● Connected' : '○ Disconnected');
    connection.dataset.connected = String(state.connected);
    this.statusLine!.append(connection,
      this.node(doc, 'strong', '', `${state.band || 'Out of band'} · ${(state.frequency / 1e6).toFixed(6)} MHz · ${state.mode}`),
      this.node(doc, 'span', 'sw-profile', state.profile ? `Profile: ${state.profile}` : 'Current settings'),
      this.node(doc, 'span', 'sw-tx-state', state.tx));
    for (const refresh of this.refreshers) refresh();
  }
}
