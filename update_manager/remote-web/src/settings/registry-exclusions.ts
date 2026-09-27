/**
 * Interactive template controls that deliberately have no registry entry.
 *
 * The registry test requires every interactive element id in the template to be
 * either indexed in the registry or listed here with a reason, so "not indexed"
 * can never be an accident. Stale entries (ids that no longer exist) also fail.
 */

export type NonSettingReason =
  | 'navigation'          // chrome for a menu, sheet, tab strip or modal
  | 'duplicate'           // per-tier or quick-action duplicate of an indexed control
  | 'child-of-indexed';   // parameter inside a control surface that is already indexed

export type NonSettingControl = {
  id: string;
  reason: NonSettingReason;
  /** The indexed entry that covers this control, when it is a duplicate or child. */
  coveredBy?: string;
  note: string;
};

const control = (value: NonSettingControl): NonSettingControl => value;

export const NON_SETTING_CONTROLS: readonly NonSettingControl[] = [
  // Phone overflow menu chrome (Phase 2 component).
  control({ id: 'phone-menu-btn', reason: 'navigation', note: 'Trigger for the phone overflow sheet.' }),
  control({ id: 'phone-menu-back-btn', reason: 'navigation', note: 'Returns from the hosted System view.' }),
  control({ id: 'phone-menu-close-btn', reason: 'navigation', note: 'Closes the phone overflow sheet.' }),
  control({ id: 'phone-menu-status-btn', reason: 'navigation', coveredBy: 'network.status', note: 'Opens the existing operator detail overlay.' }),
  control({ id: 'phone-menu-system-btn', reason: 'navigation', coveredBy: 'about.device', note: 'Hosts the header diagnostics menu inside the sheet.' }),
  control({ id: 'phone-menu-help-btn', reason: 'navigation', coveredBy: 'interface.keyboardShortcuts', note: 'Routes to the shortcut help.' }),
  control({ id: 'phone-menu-phone-btn', reason: 'navigation', coveredBy: 'interface.phoneView', note: 'Routes to the existing Phone view toggle.' }),
  control({ id: 'phone-menu-theme-btn', reason: 'navigation', coveredBy: 'interface.theme', note: 'Routes to the existing theme control.' }),
  control({ id: 'phone-menu-settings-btn', reason: 'navigation', coveredBy: 'shell.settings', note: 'Routes to the existing settings entry point.' }),
  control({ id: 'phone-menu-radio-path-btn', reason: 'navigation', coveredBy: 'radio.antenna', note: 'Routes to the existing radio-path panel.' }),
  control({ id: 'phone-menu-ops-memory-btn', reason: 'navigation', coveredBy: 'memory.operationsTab', note: 'Operations route.' }),
  control({ id: 'phone-menu-ops-audio-btn', reason: 'navigation', coveredBy: 'audio.operationsTab', note: 'Operations route.' }),
  control({ id: 'phone-menu-ops-network-btn', reason: 'navigation', coveredBy: 'network.operationsTab', note: 'Operations route.' }),
  control({ id: 'phone-menu-ops-dsp-btn', reason: 'navigation', coveredBy: 'receive.dspTab', note: 'Operations route.' }),
  control({ id: 'phone-menu-ops-radio-btn', reason: 'navigation', coveredBy: 'radio.operationsTab', note: 'Operations route.' }),
  control({ id: 'phone-menu-ops-log-btn', reason: 'navigation', coveredBy: 'log.liveTrace', note: 'Operations route.' }),
  control({ id: 'system-go-live-btn', reason: 'duplicate', coveredBy: 'network.goLive', note: 'System-menu entry forwards to the original Go Live, Cancel Connect or Cancel Retry handler.' }),

  // Legacy settings shell chrome.
  control({ id: 'setup-menu-btn', reason: 'navigation', coveredBy: 'shell.settings', note: 'Legacy settings trigger.' }),
  control({ id: 'settings-search', reason: 'navigation', coveredBy: 'shell.settings', note: 'Search field for the Settings index.' }),
  control({ id: 'settings-index-return', reason: 'navigation', coveredBy: 'shell.settings', note: 'Returns from an original setup panel to the Settings section index.' }),
  control({ id: 'setup-close-btn', reason: 'navigation', note: 'Closes the legacy settings sheet.' }),
  control({ id: 'setup-tab-profiles', reason: 'navigation', coveredBy: 'radio.profileSelect', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-display', reason: 'navigation', coveredBy: 'display.view', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-dsp', reason: 'navigation', coveredBy: 'receive.nr', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-tx', reason: 'navigation', coveredBy: 'transmit.audioSource', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-network', reason: 'navigation', coveredBy: 'network.wsUrl', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-audio', reason: 'navigation', coveredBy: 'audio.inputSelect', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-tab-advanced', reason: 'navigation', coveredBy: 'radio.sampleRate', note: 'Section tab in the legacy sheet.' }),
  control({ id: 'setup-dsp-tab-nr', reason: 'navigation', coveredBy: 'receive.nr', note: 'Sub-tab inside the DSP section.' }),
  control({ id: 'setup-dsp-tab-anf', reason: 'navigation', coveredBy: 'receive.anf', note: 'Sub-tab inside the DSP section.' }),
  control({ id: 'setup-dsp-tab-eq', reason: 'navigation', coveredBy: 'receive.rxEq', note: 'Sub-tab inside the DSP section.' }),
  control({ id: 'setup-audio-open-rx-btn', reason: 'navigation', coveredBy: 'receive.startStop', note: 'Opens the receive audio panel.' }),
  control({ id: 'setup-audio-open-tx-btn', reason: 'navigation', coveredBy: 'transmit.audioSource', note: 'Opens the transmit audio panel.' }),

  // Modal chrome.
  control({ id: 'operations-close-btn', reason: 'navigation', coveredBy: 'shell.operationsDrawer', note: 'Closes the operations drawer.' }),
  control({ id: 'operations-audio-close-btn', reason: 'navigation', coveredBy: 'audio.scope', note: 'Collapses the audio scope panel.' }),
  control({ id: 'operator-detail-close-btn', reason: 'navigation', coveredBy: 'network.status', note: 'Closes the operator detail overlay.' }),
  control({ id: 'operator-detail-copy-rx-btn', reason: 'duplicate', coveredBy: 'network.diagnostics', note: 'Copies the same operator log as the diagnostics button.' }),
  control({ id: 'shortcut-close-btn', reason: 'navigation', coveredBy: 'interface.keyboardShortcuts', note: 'Closes the shortcut help overlay.' }),
  control({ id: 'freq-entry-cancel-btn', reason: 'navigation', coveredBy: 'radio.keypad', note: 'Cancels direct frequency entry.' }),

  // Per-tier and quick-action duplicates of indexed controls.
  control({ id: 'vfo-mode-tag', reason: 'duplicate', coveredBy: 'radio.mode', note: 'VFO mode shortcut for the mode grid.' }),
  control({ id: 'vfo-band-tag', reason: 'duplicate', coveredBy: 'radio.band', note: 'VFO band shortcut for the band grid.' }),
  control({ id: 'vfo-antenna-pill', reason: 'duplicate', coveredBy: 'radio.antenna', note: 'VFO antenna shortcut for the radio path select.' }),
  control({ id: 'rx-att-quick-btn', reason: 'duplicate', coveredBy: 'radio.attenuator', note: 'VFO-line shortcut for the same attenuator value.' }),
  control({ id: 'mobile-att-btn', reason: 'duplicate', coveredBy: 'radio.attenuator', note: 'Phone-view quick action.' }),
  control({ id: 'mobile-nr-btn', reason: 'duplicate', coveredBy: 'receive.nr', note: 'Phone-view quick action.' }),
  control({ id: 'mobile-anf-btn', reason: 'duplicate', coveredBy: 'receive.anf', note: 'Phone-view quick action.' }),
  control({ id: 'mobile-filter-btn', reason: 'duplicate', coveredBy: 'receive.filterLow', note: 'Phone-view quick action for the RX filter.' }),
  control({ id: 'mobile-wake-lock-btn', reason: 'duplicate', coveredBy: 'display.wakeLockAction', note: 'Phone-view quick action for the wake lock.' }),

  // Parameters inside control surfaces that are already indexed.
  control({ id: 'band-more-btn', reason: 'child-of-indexed', coveredBy: 'radio.band', note: 'Reveals the remaining band choices on narrow tiers.' }),
  control({ id: 'mode-more-btn', reason: 'child-of-indexed', coveredBy: 'radio.mode', note: 'Reveals the remaining mode choices on narrow tiers.' }),
  control({ id: 'two-tone-freq1', reason: 'child-of-indexed', coveredBy: 'transmit.twoToneEnable', note: 'Two-tone generator parameter.' }),
  control({ id: 'two-tone-freq2', reason: 'child-of-indexed', coveredBy: 'transmit.twoToneEnable', note: 'Two-tone generator parameter.' }),
  control({ id: 'two-tone-level', reason: 'child-of-indexed', coveredBy: 'transmit.twoToneEnable', note: 'Two-tone generator parameter.' }),
  control({ id: 'two-tone-delay', reason: 'child-of-indexed', coveredBy: 'transmit.twoToneEnable', note: 'Two-tone generator parameter.' }),
  control({ id: 'two-tone-invert-lsb', reason: 'child-of-indexed', coveredBy: 'transmit.twoToneEnable', note: 'Two-tone generator parameter.' }),
];

export const NON_SETTING_CONTROL_IDS: ReadonlyMap<string, NonSettingControl> = new Map(
  NON_SETTING_CONTROLS.map((item) => [item.id, item]),
);
