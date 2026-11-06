import { SETTINGS_REGISTRY, type SettingsEntry } from './registry';

export const WORKBENCH_PAGES = [
  { id: 'radio', label: 'Radio & bands', icon: '◉', detail: 'Tuning, signal paths and saved profiles' },
  { id: 'receive', label: 'Receiver', icon: '↓', detail: 'Passband, gain, equalizer and noise processing' },
  { id: 'transmit', label: 'Transmitter', icon: '↑', detail: 'Levels, ESSB processing and test generator' },
  { id: 'ps', label: 'PureSignal', icon: '∿', detail: 'Adaptation, feedback and measurement tools' },
  { id: 'display', label: 'Display', icon: '▥', detail: 'Spectrum, waterfall and 3D terrain' },
  { id: 'audio', label: 'Audio devices', icon: '♫', detail: 'Microphone, speakers and audio transport' },
  { id: 'meter', label: 'Meters', icon: '◴', detail: 'Signal strength, power and meter response' },
  { id: 'network', label: 'Connection', icon: '⇄', detail: 'Bridge, transport and connection diagnostics' },
  { id: 'interface', label: 'Appearance & layout', icon: '▦', detail: 'Theme, main-screen layout and shortcuts' },
  { id: 'about', label: 'System & logs', icon: 'ⓘ', detail: 'Identity, diagnostic reports and client log' },
] as const;
export type WorkbenchPage = typeof WORKBENCH_PAGES[number]['id'];

/** Presentation only: every editable item still resolves to an existing control. */
export function workbenchPage(entry: SettingsEntry): WorkbenchPage {
  if (/^transmit\.(?:ps|puresignal)/i.test(entry.id)) return 'ps';
  if (entry.section === 'memory') return 'radio';
  if (entry.section === 'log') return 'about';
  return entry.section;
}

export function workbenchGroup(entry: SettingsEntry): string {
  if (entry.targetKind === 'query-param') return 'Launch options';
  if (entry.targetKind === 'storage-key') return 'Saved preferences';
  const id = entry.id.toLowerCase();
  switch (workbenchPage(entry)) {
    case 'radio': return /profile/.test(id) ? 'Profiles' : /memory|band/.test(id) ? 'Bands & memories' : 'Tuning & signal paths';
    case 'receive': return /nr|anr/.test(id) ? 'Noise reduction' : /anf|\.nb|blanker/.test(id) ? 'Notch & noise blanker' : /filter|passband|\.rxeq/.test(id) ? 'Passband & equalizer' : 'Audio & gain';
    case 'transmit': return /twotone/.test(id) ? 'Two-tone generator' : /eq|cfc/.test(id) ? 'Equalizer & CFC' : /gate|dexp|speech|cessb|phaserotator/.test(id) ? 'Voice processing' : /monitor|timeout/.test(id) ? 'Monitor & timing' : /state|power|swr|countdown|miclevel|\.arm$|\.ptt$|\.mox$|\.lock$/.test(id) ? 'Operating status' : 'Source, level & passband';
    case 'ps': return /status|feedback|calibrations|correcting|packets|gaps|timing|contract|result/i.test(id) ? 'Host reports' : /measure|stimulus|watts|duration/i.test(id) ? 'Measurement tools' : /attenuation|attenuate/.test(id) ? 'Feedback path' : 'Adaptation & calibration';
    case 'display': return /3d|terrain|colormap|split|history|diagnostics/.test(id) ? '3D terrain' : /waterfall/.test(id) ? 'Waterfall' : 'Spectrum';
    case 'audio': return /device|input|output/.test(id) ? 'Devices' : 'Audio health & transport';
    case 'meter': return 'Meter presentation';
    case 'network': return 'Bridge & transport';
    case 'interface': return 'Appearance & main screen';
    case 'about': return /log/.test(id) ? 'Operator & client logs' : 'System & diagnostics';
  }
}

export function settingScope(entry: SettingsEntry): string {
  // Scope describes the existing persistence contract, never creates a new one.
  if (['query-param', 'storage-key'].includes(entry.targetKind)) return entry.targetKind === 'query-param' ? 'On launch' : 'Browser';
  if (['readout', 'indicator'].includes(entry.targetKind)) return 'Live status';
  if (entry.section === 'display' && !/view|pause|status|history|diagnostic|clarity|appearance|center|awake|lock|toolbar/i.test(entry.id)
    && !entry.id.startsWith('display.3d.') && !/terrain|colormap|split/.test(entry.id)) return 'Per band';
  if (entry.section === 'receive' && !/startStop|buffer|codec|dspTab|passband/.test(entry.id)) return 'Per band';
  if (entry.section === 'transmit' && /^(transmit\.(?:drive|micGain|filter|eq|cfc|noiseGate|dexp|speechProcessor|cessb|phaseRotator|monitor|timeout|twoTone(?:Freq|Level|Delay|Invert)|puresignal(?:Enabled|AutoAttenuate|Attenuation)?$|psAdaptation))/.test(entry.id)) return 'Per band';
  if (entry.section === 'radio' && /antenna|attenuator|sampleRate|rxAdc|mode|wbfm/i.test(entry.id)) return 'Per band';
  return ['button', 'overlay', 'toggle'].includes(entry.targetKind) ? 'Control' : 'Browser / session';
}

const extra = (id: string, section: SettingsEntry['section'], label: string, targetId: string, targetKind: SettingsEntry['targetKind'], description: string): SettingsEntry => ({
  id, section, label, targetId, targetKind, description, synonyms: [], mainScreen: 'settings-only', parityStatus: 'wired',
});

// Detailed controls already implemented by the radio UI but previously absent
// from its Settings index. EQ/CFC bands are discovered after their grids mount.
export const WORKBENCH_EXTRAS: readonly SettingsEntry[] = [
  extra('transmit.twoToneFreq1', 'transmit', 'Tone 1 frequency', 'two-tone-freq1', 'slider', 'First test-generator frequency, in Hz.'),
  extra('transmit.twoToneFreq2', 'transmit', 'Tone 2 frequency', 'two-tone-freq2', 'slider', 'Second test-generator frequency, in Hz.'),
  extra('transmit.twoToneLevel', 'transmit', 'Generator level', 'two-tone-level', 'slider', 'Generator level in dB; uses the existing TX source.'),
  extra('transmit.twoToneDelay', 'transmit', 'Generator delay', 'two-tone-delay', 'slider', 'Delay after keying, in milliseconds.'),
  extra('transmit.twoToneInvert', 'transmit', 'Invert lower sideband tones', 'two-tone-invert-lsb', 'toggle', 'Use the existing LSB/DIGL/CWL tone orientation.'),
  extra('transmit.psStimulus', 'transmit', 'Measurement stimulus', 'setup-ps-measurement-kind', 'select', 'Choose the two-tone/carrier or timing measurement.'),
  extra('transmit.psWatts', 'transmit', 'Measurement power target', 'setup-ps-commanded-watts', 'slider', 'Commanded watts; this is not a measured power reading.'),
  extra('transmit.psDuration', 'transmit', 'Measurement duration', 'setup-ps-duration-ms', 'slider', 'Uses the measurement duration accepted by the current host.'),
  extra('transmit.psTransport', 'transmit', 'TX owner selection', 'managed-pcm-select', 'select', 'Select the transport while disconnected, as on the PS tab.'),
  extra('transmit.psStatus', 'transmit', 'Native state', 'setup-tx-puresignal-state', 'readout', 'The current host WDSP report.'),
  extra('transmit.psFeedback', 'transmit', 'Feedback level', 'setup-tx-puresignal-feedback', 'readout', 'Raw WDSP feedback level, not percent, watts or dBm.'),
  extra('transmit.psCalibrations', 'transmit', 'Calibration count', 'setup-tx-puresignal-calibrations', 'readout', 'Calibrations reported by the native engine.'),
  extra('transmit.psCorrecting', 'transmit', 'Correction state', 'setup-tx-puresignal-correcting', 'readout', 'Host correction report; not a calibrated RF IMD result.'),
  extra('transmit.psPackets', 'transmit', 'Feedback packets', 'setup-ps-packets', 'readout', 'Packet count reported for the feedback stream.'),
  extra('transmit.psGaps', 'transmit', 'Feedback gaps', 'setup-ps-gaps', 'readout', 'Feedback discontinuities reported by the host.'),
  extra('transmit.psTiming', 'transmit', 'Report timing', 'setup-ps-timing', 'readout', 'Existing host-report freshness information.'),
  extra('transmit.psContract', 'transmit', 'Host capabilities', 'setup-ps-control-contract', 'readout', 'Available controls follow the negotiated host capabilities.'),
  extra('transmit.psResult', 'transmit', 'Measurement result', 'setup-ps-measurement-result', 'readout', 'Latest measurement request and saved-capture result.'),
  extra('transmit.psMeasurementStop', 'transmit', 'Stop measurement', 'setup-ps-measure-stop', 'button', 'Release the current measurement using the existing TX owner.'),
  extra('transmit.psTransactionStatus', 'transmit', 'Settings transaction', 'managed-pcm-transaction', 'readout', 'Queued, applied and rejected settings results from the host.'),
];

export function workbenchEntries(doc: Document): SettingsEntry[] {
  const entries = [...SETTINGS_REGISTRY, ...WORKBENCH_EXTRAS.filter(entry => doc.getElementById(entry.targetId))];
  for (const [grid, section, prefix, name] of [
    ['rx-eq-grid', 'receive', 'receive.rxEqBand', 'RX equalizer'],
    ['tx-eq-grid', 'transmit', 'transmit.eqBand', 'TX equalizer'],
    ['cfc-band-grid', 'transmit', 'transmit.cfcBand', 'CFC'],
  ] as const) {
    doc.getElementById(grid)?.querySelectorAll<HTMLInputElement>('input[id]').forEach((input, i) => {
      const label = input.closest('.eq-band')?.querySelector('label, .eq-freq, .eq-band-label, .eq-label')?.textContent?.trim()
        || input.getAttribute('aria-label') || `Band ${i + 1}`;
      const frequency = /^[0-9.]+k?$/i.test(label) ? `${label} Hz` : label;
      entries.push(extra(`${prefix}${i}`, section, `${name} · ${frequency}`, input.id, 'slider', `${name} band level in dB.`));
    });
  }
  return entries;
}
