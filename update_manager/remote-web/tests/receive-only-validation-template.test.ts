import { parse } from 'acorn';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createContext, runInContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const template = readFileSync(resolve(process.cwd(), '../templates/saturn-remote-next.html'), 'utf8');
const BEGIN = '/* RECEIVE_ONLY_GUARD_BEGIN */';
const END = '/* RECEIVE_ONLY_GUARD_END */';
const block = template.slice(template.indexOf(BEGIN), template.indexOf(END) + END.length);
const guardFile = readFileSync(resolve(process.cwd(), '../scripts/saturn-remote-receive-guard.js'), 'utf8');
const guardFileBlock = guardFile.slice(guardFile.indexOf(BEGIN), guardFile.indexOf(END) + END.length);

/** What a normal Go Live from a fresh page sends (recorded against the real page and Bridge): it retunes. */
const GO_LIVE_FRAMES = [
  'audio_gain:client;',
  'rx_smeter:0,0;',
  'tx_codec_caps:0,pcm;',
  'tx_noise_gate:0,true;tx_noise_gate_threshold:0,-30.0;tx_eq_enable:0,true;tx_cfc_enable:0,true;tx_cfc_precomp:0,1.0;tx_eq_band:0,1,0;',
  'saturn_ping:1,4129.625;',
  'saturn_display:iq;',
  'audio_codec:opus;',
  'iq_samplerate:384000;rx_adc:0,0;rx_antenna:0,1;modulation:0,USB;rx_volume:0,0,-10.0;rx_nr_mode:0,OFF;rx_nr_level:0,0;',
  'vfo:0,0,14200000;dds:0,14200000;',
  'iq_samplerate:384000;',
  'iq_start:0;',
  'audio_stream_samples:2048;audio_stream_channels:2;audio_stream_sample_type:float32;audio_samplerate:48000;audio_start:0;rx_volume:0,0,-10.0;',
];

type GuardModule = {
  ALLOWED_COMMANDS: readonly string[];
  requested: (search: string) => boolean;
  filterText: (text: string) => { text: string; blocked: string[] };
  newRecorder: () => Recorder;
  recorderFor: (win: unknown) => Recorder;
  install: (win: unknown) => Recorder;
};

type Recorder = {
  blockedCounts: Record<string, number>;
  blockedExamples: string[];
  violations: { kind: string; detail: string }[];
  errors: { kind: string; detail: string }[];
};

/** A fresh WebSocket class per page load: the guard patches the prototype, and a shared class would carry one test's guard into the next. */
function makeFakeWebSocket() {
  return class FakeWebSocket {
    static sent: unknown[] = [];
    send(data: unknown) {
      FakeWebSocket.sent.push(data);
    }
  };
}

function fakeWindow(search: string, FakeWebSocket: ReturnType<typeof makeFakeWebSocket>) {
  const listeners: Record<string, ((event: unknown) => void)[]> = {};
  const fetched: { input: unknown; init: unknown }[] = [];
  const xhrOpened: string[] = [];
  class FakeXhr {
    open(method: string) {
      xhrOpened.push(method);
    }
  }
  const win: Record<string, any> = {
    location: { search },
    WebSocket: FakeWebSocket,
    fetch: (input: unknown, init: unknown) => {
      fetched.push({ input, init });
      return Promise.resolve({ ok: true });
    },
    navigator: { sendBeacon: () => true },
    XMLHttpRequest: FakeXhr,
    addEventListener: (type: string, fn: (event: unknown) => void) => {
      (listeners[type] ||= []).push(fn);
    },
  };
  return { win, listeners, fetched, xhrOpened };
}

/** The page's wiring: the module is created, and installed only when ?validation=receive asks for it. */
function load(search: string) {
  const FakeWebSocket = makeFakeWebSocket();
  const w = fakeWindow(search, FakeWebSocket);
  const context = createContext({ window: w.win, URLSearchParams, Set, Object, String, Promise, TypeError, Error });
  const module = runInContext(`${block}\ncreateReceiveOnlyGuardModule()`, context) as GuardModule;
  const session = module.requested(search) ? module.install(w.win) : null;
  return { ...module, ...w, session, module, sent: () => FakeWebSocket.sent };
}

/** The injected file, run the way the validator runs it: before any page script, in a window that has no knowledge of validation mode. */
function injected(search = '') {
  const FakeWebSocket = makeFakeWebSocket();
  const w = fakeWindow(search, FakeWebSocket);
  const context = createContext({ window: w.win, URLSearchParams, Set, Object, String, Promise, TypeError, Error });
  runInContext(guardFile, context);
  return { ...w, sent: () => FakeWebSocket.sent, FakeWebSocket };
}

describe('receive-only validation mode: the request', () => {
  it('is on only for ?validation=receive', () => {
    const { requested } = load('');
    expect(requested('?validation=receive')).toBe(true);
    expect(requested('?transport=split&validation=receive&display_transport=spectrum')).toBe(true);
    for (const search of ['', '?validation=1', '?validation=Receive', '?validation=', '?validate=receive', '?x=validation=receive']) {
      expect(requested(search), search).toBe(false);
    }
  });

  it('installs nothing on a normal page', () => {
    const normal = load('?transport=split');
    expect(normal.session).toBeNull();
    expect(normal.win.WebSocket.prototype.send.receiveOnlyGuard).toBeUndefined();
    expect(normal.win.saturnReceiveGuardRecorder).toBeUndefined();
    expect(normal.win.fetch.receiveOnlyGuard).toBeUndefined();
    new normal.win.WebSocket().send('vfo:0,0,7074000;');
    expect(normal.sent()).toEqual(['vfo:0,0,7074000;']);
  });
});

describe('receive-only validation mode: the command allowlist', () => {
  it('lets a viewer-class session negotiate audio and display and nothing else', () => {
    const { filterText: filter } = load('');
    const sent: string[] = [];
    const blocked: string[] = [];
    for (const frame of GO_LIVE_FRAMES) {
      const result = filter(frame);
      sent.push(...result.text.split(';').filter(Boolean));
      blocked.push(...result.blocked);
    }
    expect(sent).toEqual([
      'audio_gain:client', 'saturn_ping:1,4129.625', 'saturn_display:iq', 'audio_codec:opus', 'iq_start:0',
      'audio_stream_samples:2048', 'audio_stream_channels:2', 'audio_stream_sample_type:float32', 'audio_samplerate:48000', 'audio_start:0',
    ]);
    const names = new Set(blocked.map((c) => c.split(':')[0]));
    for (const name of ['vfo', 'dds', 'iq_samplerate', 'rx_adc', 'rx_antenna', 'modulation', 'rx_volume', 'rx_nr_mode', 'rx_nr_level',
      'rx_smeter', 'tx_codec_caps', 'tx_noise_gate', 'tx_noise_gate_threshold', 'tx_eq_enable', 'tx_cfc_enable', 'tx_cfc_precomp', 'tx_eq_band']) {
      expect(names.has(name), name).toBe(true);
    }
  });

  it('denies by default: unknown, malformed, empty and case-varied commands', () => {
    const { filterText: filter } = load('');
    expect(filter('').text).toBe('');
    expect(filter(';;;').text).toBe('');
    expect(filter('nonsense').blocked).toEqual(['nonsense']);
    expect(filter('VFO:0,0,1;').blocked).toEqual(['VFO:0,0,1']);
    expect(filter('trx:0,true;').text).toBe('');
    expect(filter('saturn_satp_control:renew;').text).toBe('');
    expect(filter('AUDIO_START:0;').text).toBe('AUDIO_START:0;');          // names compare case-insensitively, as the Bridge does
    expect(filter('audio_start:0;vfo:0,0,1;audio_stop:0').text).toBe('audio_start:0;audio_stop:0;');
  });

  it('allows nothing the Bridge would refuse a viewer (cross-checked against the Rust source)', () => {
    const rust = readFileSync(resolve(process.cwd(), '../saturn-bridge/src/tci/protocol.rs'), 'utf8');
    const body = rust.slice(rust.indexOf('pub(crate) fn viewer_tci_command_allowed'));
    const viewerList = body.slice(0, body.indexOf('\n}\n'));
    const bridgeAllowed = new Set([...viewerList.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]));
    expect(bridgeAllowed.size).toBeGreaterThan(10);
    const { ALLOWED_COMMANDS: pageAllowed } = load('');
    const outside = pageAllowed.filter((name) => name !== 'session_open' && !bridgeAllowed.has(name));
    expect(outside, 'page allows commands the Bridge refuses a viewer').toEqual([]);
    // session_open is the split transport's own handshake, handled before the viewer check
    expect(rust.includes('"session_open"') || readFileSync(resolve(process.cwd(), '../saturn-bridge/src/tci/session_pair.rs'), 'utf8').includes('session_open')).toBe(true);
  });

  it('can never allow a radio-changing command', () => {
    const { ALLOWED_COMMANDS: allowed } = load('');
    for (const name of allowed) {
      expect(name, name).not.toMatch(/^(vfo|dds|modulation|trx|drive|tune|rx_|tx_|iq_samplerate|split|agc|rit|xit|cw|volume|mute|mox|tune)/);
    }
  });
});

describe('receive-only validation mode: the viewer role', () => {
  it('opens every session as a viewer, whatever role the page asked for', () => {
    const { filterText: filter } = load('');
    expect(filter('session_open:split-abc,operator;').text).toBe('session_open:split-abc,viewer;');
    expect(filter('session_open:split-abc,viewer;').text).toBe('session_open:split-abc,viewer;');
    expect(filter('session_open:split-abc,OPERATOR;').text).toBe('session_open:split-abc,viewer;');
    expect(filter('session_open:split-abc;').text).toBe('session_open:split-abc,viewer;');
    expect(filter('SESSION_OPEN:split-abc,operator;').text).toBe('session_open:split-abc,viewer;');
  });
});

describe('receive-only validation mode: the guard file the validator injects', () => {
  it('is byte-identical to the copy inside the page', () => {
    expect(guardFileBlock.length).toBeGreaterThan(2000);
    expect(guardFileBlock).toBe(block);
  });

  it('leaves no global binding that a page script could collide with', () => {
    const context = createContext({ window: fakeWindow('', makeFakeWebSocket()).win, URLSearchParams, Set, Object, String, Promise, TypeError, Error });
    runInContext(guardFile, context);
    // a page declares its own top-level names after the injected file ran; none of them may already exist
    expect(() => runInContext('const createReceiveOnlyGuardModule = 1; const receiveOnlyGuard = 2; const receiveOnlyValidationSession = 3;', context)).not.toThrow();
  });

  it('protects an old page that knows nothing about validation mode: role forced to viewer, radio commands dropped, no uplink', () => {
    const g = injected('');
    const ws = new g.win.WebSocket();
    ws.send('session_open:split-old,operator;');
    ws.send('vfo:0,0,7074000;dds:0,7074000;iq_samplerate:384000;modulation:0,USB;trx:0,true;');
    ws.send('iq_start:0;audio_start:0;');
    ws.send(new Uint8Array(1280).buffer);
    expect(g.sent()).toEqual(['session_open:split-old,viewer;', 'iq_start:0;audio_start:0;']);
    const recorder = g.win.saturnReceiveGuardRecorder as Recorder;
    expect(recorder.blockedCounts).toEqual({ vfo: 1, dds: 1, iq_samplerate: 1, modulation: 1, trx: 1 });
    expect(recorder.violations.map((x) => x.kind)).toEqual(['binary-send']);
  });

  it('blocks writes to the server from an old page too', async () => {
    const g = injected('');
    await expect(g.win.fetch('/remote_settings', { method: 'POST', body: '{}' })).rejects.toThrow();
    await expect(g.win.fetch('/remote_settings')).resolves.toBeTruthy();
    expect(g.fetched.length).toBe(1);
  });

  it('shares one recorder with the page copy, so the page snapshot sees what the injected guard blocked', () => {
    const FakeWebSocket = makeFakeWebSocket();
    const w = fakeWindow('?validation=receive', FakeWebSocket);
    const context = createContext({ window: w.win, URLSearchParams, Set, Object, String, Promise, TypeError, Error });
    runInContext(guardFile, context);                                    // injected first, as the validator does
    new w.win.WebSocket().send('vfo:0,0,1;');
    const pageModule = runInContext(`${block}\ncreateReceiveOnlyGuardModule()`, context) as GuardModule;
    const pageRecorder = pageModule.install(w.win);                      // then the page's own wiring
    expect(pageRecorder).toBe(w.win.saturnReceiveGuardRecorder);
    expect(pageRecorder.blockedCounts).toEqual({ vfo: 1 });
    expect(FakeWebSocket.sent).toEqual([]);
  });
});

describe('receive-only validation mode: the guards on a validation page', () => {
  it('filters text sends at WebSocket.send and sends the allowed part', () => {
    const v = load('?validation=receive');
    expect(v.session).not.toBeNull();
    const ws = new v.win.WebSocket();
    ws.send('vfo:0,0,14200000;dds:0,14200000;');
    expect(v.sent()).toEqual([]);
    ws.send('iq_start:0;vfo:0,0,1;audio_start:0;');
    expect(v.sent()).toEqual(['iq_start:0;audio_start:0;']);
    ws.send('session_open:split-abc,operator;');
    expect(v.sent().at(-1)).toBe('session_open:split-abc,viewer;');
    expect(v.session!.blockedCounts).toEqual({ vfo: 2, dds: 1 });
  });

  it('never sends a binary frame (no uplink media)', () => {
    const v = load('?validation=receive');
    const ws = new v.win.WebSocket();
    ws.send(new Uint8Array(640).buffer);
    ws.send(new Uint8Array(32));
    expect(v.sent()).toEqual([]);
    expect(v.session!.violations.map((x) => x.kind)).toEqual(['binary-send', 'binary-send']);
    expect(v.session!.violations[0]!.detail).toBe('640 bytes');
  });

  it('lets GET and HEAD through and refuses everything that could write', async () => {
    const v = load('?validation=receive');
    await v.win.fetch('/remote_settings', { cache: 'no-store' });
    await v.win.fetch('/readyz', { method: 'head' });
    expect(v.fetched.length).toBe(2);
    for (const method of ['POST', 'put', 'DELETE', 'PATCH']) {
      await expect(v.win.fetch('/remote_profiles/save', { method })).rejects.toThrow(/not permitted/);
    }
    expect(v.fetched.length).toBe(2);
    expect(v.session!.violations.map((x) => x.kind)).toEqual(['fetch-POST', 'fetch-PUT', 'fetch-DELETE', 'fetch-PATCH']);
    expect(v.win.navigator.sendBeacon('/x')).toBe(false);
    expect(v.session!.violations.at(-1)!.kind).toBe('beacon');
    const xhr = new v.win.XMLHttpRequest();
    xhr.open('GET', '/a');
    expect(() => xhr.open('POST', '/b')).toThrow(/not permitted/);
    expect(v.xhrOpened).toEqual(['GET']);
  });

  it('records uncaught errors for the validator to see', () => {
    const v = load('?validation=receive');
    v.listeners.error![0]!({ message: 'boom' });
    v.listeners.unhandledrejection![0]!({ reason: new Error('rejected') });
    expect(v.session!.errors).toEqual([{ kind: 'uncaught', detail: 'boom' }, { kind: 'unhandled-rejection', detail: 'rejected' }]);
  });

  it('installs once even if loaded twice', () => {
    const v = load('?validation=receive');
    const sendBefore = v.win.WebSocket.prototype.send;
    v.install(v.win);
    expect(v.win.WebSocket.prototype.send).toBe(sendBefore);
    expect(v.recorderFor(v.win)).toBe(v.session);
  });
});

describe('receive-only validation mode: the page wiring', () => {
  const pageScript = [...template.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)]
    .map((match) => match[1] ?? '')
    .find((script) => script.includes('function startRxSpectrumCapture'))!;

  it('the whole page script still parses', () => {
    expect(() => parse(pageScript, { ecmaVersion: 'latest' })).not.toThrow();
  });

  it('every function that reads or writes saved settings or profiles returns first in a validation session', () => {
    for (const header of [
      'async function loadRemoteSettingsFromServer(logFailures = false) {',
      'async function loadRemoteProfilesFromServer(logFailures = false) {',
      'async function saveRemoteSettingsToServer(logFailures = false) {',
      'async function saveProfileToServer(name, makeStartup = false) {',
      'async function deleteProfileFromServer(name) {',
      'async function setStartupProfileOnServer(name) {',
      'function applyProfileByName(name, persist = true) {',
    ]) {
      const at = pageScript.indexOf(header);
      expect(at, header).toBeGreaterThan(-1);
      expect(pageScript.slice(at + header.length, at + header.length + 80), header).toMatch(/^\s*if \(receiveOnlyValidationSession\) return false;/);
    }
  });

  it('every request to a saved-settings or profile endpoint sits inside one of those gated functions', () => {
    const gated = ['loadRemoteSettingsFromServer', 'loadRemoteProfilesFromServer', 'saveRemoteSettingsToServer', 'saveProfileToServer', 'deleteProfileFromServer', 'setStartupProfileOnServer'];
    const body = parse(pageScript, { ecmaVersion: 'latest' }) as unknown as { body: { type: string; id?: { name: string }; start: number; end: number }[] };
    const functions = body.body.filter((n) => n.type === 'FunctionDeclaration' && n.id);
    for (const path of ['REMOTE_SETTINGS_PATH', 'REMOTE_PROFILES_PATH', 'REMOTE_PROFILES_SAVE_PATH', 'REMOTE_PROFILES_DELETE_PATH', 'REMOTE_PROFILES_STARTUP_PATH']) {
      for (const match of pageScript.matchAll(new RegExp(`fetch\\(${path}`, 'g'))) {
        const owner = functions.find((f) => match.index! >= f.start && match.index! < f.end);
        expect(gated, `${path} is fetched from ${owner?.id?.name}`).toContain(owner?.id?.name);
      }
    }
  });

  it('asks the Bridge for the viewer role, and refuses to start without the split transport that carries it', () => {
    expect(pageScript).toContain('role: receiveOnlyValidationSession ? "viewer" : undefined');
    expect(pageScript).toContain('const receiveOnlyGuard = createReceiveOnlyGuardModule();');
    expect(pageScript).toContain('? receiveOnlyGuard.install(window)');
    const hook = pageScript.slice(pageScript.indexOf('window.saturnValidation = {'));
    expect(hook).toContain('if (!splitTransportEnabled()) return { ok: false');
    expect(hook).toContain('beginBridgeSession();');
    expect(hook).toContain('await disconnectBridge();');
  });

  it('the snapshot reports what the validator needs, and only those facts', () => {
    const hook = pageScript.slice(pageScript.indexOf('snapshot() {'), pageScript.indexOf('snapshot() {') + 2200);
    for (const field of ['connected', 'bridgeReady', 'role', 'audioContextState', 'codec', 'opusFrames', 'decodeErrors', 'audioFramesPlayed', 'displayRows', 'blockedCounts', 'violations', 'errors']) {
      expect(hook, field).toContain(field);
    }
  });
});
