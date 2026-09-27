import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

// Phase 3 acceptance gate: every path that can end a transmission must actually
// end it. This drives the real template handlers (not copies) through each
// release route and asserts the transmitter is left idle afterwards.
//
// Scope note: the socket-close path lives in the connection state machine, not in
// the PTT binding module, so it is asserted structurally at the end rather than
// by dispatching events here.
const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

function slice(startMarker: string, endMarker: string): string {
  const start = template.indexOf(startMarker);
  const end = template.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error(`Missing template slice: ${startMarker}`);
  return template.slice(start, end);
}

const CODE = [
  slice('    function clearPointerPttTracking() {', '    function bindKeyboardPttControls() {'),
  slice('    function normalizeTxReadyState(', '    function txReadyRemainingMs('),
  slice('    function lockTx(', '    function resetTxUplinkStats('),
  slice('    async function setPtt(', '    async function setTwoToneEnabled('),
  slice('    function bindPttButton() {', '    async function goLive() {'),
  '({ bindPttButton, setPtt, lockTx, holdPttActive, clearPointerPttTracking })',
].join('\n');

type FakeElement = {
  id: string;
  handlers: Record<string, ((event: Record<string, unknown>) => void)[]>;
  disabled: boolean;
  offsetParent: unknown;
  hidden: boolean;
  visibilityState: string;
  addEventListener: (type: string, handler: (event: Record<string, unknown>) => void) => void;
  removeEventListener: () => void;
  dispatch: (type: string, event?: Record<string, unknown>) => void;
  setPointerCapture: () => void;
  releasePointerCapture: () => void;
  click: () => void;
  focus: () => void;
};

function txReleaseHarness() {
  const elements: Record<string, FakeElement> = {};
  function element(id: string): FakeElement {
    if (elements[id]) return elements[id];
    const handlers: Record<string, ((event: Record<string, unknown>) => void)[]> = {};
    const node: FakeElement = {
      id,
      handlers,
      disabled: false,
      offsetParent: {},
      hidden: false,
      visibilityState: 'visible',
      addEventListener(type, handler) {
        (handlers[type] = handlers[type] || []).push(handler);
      },
      removeEventListener() {},
      dispatch(type, event = {}) {
        for (const handler of handlers[type] || []) {
          handler({ preventDefault() {}, stopPropagation() {}, ...event });
        }
      },
      setPointerCapture() {},
      releasePointerCapture() {},
      click() { this.dispatch('click', {}); },
      focus() {},
    };
    elements[id] = node;
    return node;
  }

  const state: Record<string, unknown> = {
    connected: true,
    ws: { readyState: 1 },
    remoteClientRole: 'operator',
    bridgeReady: true,
    audioStreaming: true,
    // Start armed but idle: a press must be what keys the radio.
    txPhase: 'rx',
    txEnabled: false,
    moxRequested: false,
    micCapturing: false,
    txReadyExpiresAt: 301_000,
    txLockReason: '',
    pointerPttActive: false,
    pointerPttPointerId: null,
    pointerPttRequestId: 0,
    keyboardPttActive: false,
    keyboardPttRequestId: 0,
    pttRequestId: 0,
    txLocalRequestId: 0,
    txLocalRequestActive: false,
    txLocalRequestPermitExpiresAt: 0,
  };

  const commands: string[] = [];
  const faults: string[] = [];
  let clock = 1_000;
  let micStops = 0;
  let rxRecoveries = 0;
  let shutDowns = 0;

  const documentNode = element('document');
  const windowNode = element('window');
  const context = {
    state,
    performance: { now: () => (clock += 1) },
    WebSocket: { OPEN: 1, CONNECTING: 0 },
    TX_READY_WINDOW_MS: 300_000,
    TX_LOCAL_REQUEST_PERMIT_MS: 5_000,
    satpSourceRequestAt: 0,
    _next: { txUsesBrowserMic: () => false, satpTxBlockReason: () => null },
    clearInterval: () => {},
    setInterval: () => 1,
    clearTimeout: () => {},
    setTimeout: () => 1,
    $: element,
    document: documentNode,
    window: windowNode,
    navigator: { onLine: true },
    rfDisabledBlocksTx: () => false,
    clientRoleBlocksTx: () => false,
    sendTci: (command: string) => { commands.push(command); },
    stopMicCapture: () => { micStops += 1; state.micCapturing = false; },
    recoverRxAfterMox: () => { rxRecoveries += 1; },
    applyTxOpusLabCfcGuard: () => false,
    syncPureSignalOperatorTelemetry: () => {},
    recordOperatorFault: (label: string) => { faults.push(label); },
    clearOperatorFault: () => {},
    clearTxTimeouts: () => {},
    scheduleTxReadyTimer: () => {},
    clearTxReadyTimer: () => {},
    scheduleNextBridgeRttPing: () => {},
    stopBridgeRttPings: () => {},
    releaseWakeLock: () => {},
    syncWakeLock: () => {},
    startRxAudio: () => {},
    stopRxAudio: () => {},
    resetTxOpusProducer: () => {},
    refreshTxCodecCapabilities: () => {},
    primeRxAudioContextFromGesture: async () => true,
    shouldAutoStartRxAudio: () => false,
    refreshRxLatencyTelemetry: () => {},
    setTxUplinkDegraded: () => {},
    updateTxZone: () => {},
    // The unkey command is emitted here, not as a literal trx:false in setPtt.
    sendTxShutdownIfPossible: () => { shutDowns += 1; },
    startMicCapture: async () => true,
    clearKeyboardPttTracking: () => {},
    txLockReasonLabel: (reason: string) => reason,
    clientRoleLockReason: () => 'role-pending',
    formatDurationClock: (ms: number) => `${ms}ms`,
    updateUi: () => {},
    logEvent: () => {},
    refreshBridgeControlHeartbeatForTx: () => {},
    applyTxSourceOverrides: () => {},
    setTxMediaPriority: () => {},
    muteRxAudioForMox: () => {},
    switchDisplayToTxArm: () => {},
    generatedTxSourceActive: () => false,
  };

  const api = runInNewContext(`${CODE}`, context) as {
    bindPttButton: () => void;
    setPtt: (active: boolean, options?: { useMic?: boolean }) => Promise<boolean>;
  };
  api.bindPttButton();

  async function flush(): Promise<void> {
    for (let index = 0; index < 4; index += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  }

  async function keyViaPtt(): Promise<void> {
    element('ptt-btn').dispatch('pointerdown', { pointerId: 1, pointerType: 'touch', button: 0 });
    await flush();
  }

  return {
    state,
    api,
    elements,
    element,
    documentNode,
    windowNode,
    commands,
    faults,
    keyViaPtt,
    flush,
    micStops: () => micStops,
    rxRecoveries: () => rxRecoveries,
    shutDowns: () => shutDowns,
  };
}

function expectIdleAfterRelease(harness: ReturnType<typeof txReleaseHarness>): void {
  expect(harness.state.moxRequested).toBe(false);
  expect(harness.state.txEnabled).toBe(false);
  expect(harness.state.micCapturing).toBe(false);
  expect(harness.state.pointerPttActive).toBe(false);
  expect(harness.micStops()).toBeGreaterThan(0);
  expect(harness.shutDowns()).toBeGreaterThan(0);
}

describe('TX release matrix', () => {
  it('releases on pointerup', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.element('ptt-btn').dispatch('pointerup', { pointerId: 1, pointerType: 'touch' });
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases on pointercancel', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.element('ptt-btn').dispatch('pointercancel', { pointerId: 1, pointerType: 'touch' });
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases on mouse pointerleave', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.element('ptt-btn').dispatch('pointerleave', { pointerId: 1, pointerType: 'mouse' });
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases when pointer capture is lost', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.element('ptt-btn').dispatch('lostpointercapture', { pointerId: 1 });
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases when the window loses focus during pointer PTT', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.windowNode.dispatch('blur', {});
    await h.flush();
    expectIdleAfterRelease(h);
    expect(h.faults).toContain('Pointer PTT focus lost');
  });

  it('releases when the page becomes hidden', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.documentNode.visibilityState = 'hidden';
    h.documentNode.dispatch('visibilitychange', {});
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases on pagehide', async () => {
    const h = txReleaseHarness();
    await h.keyViaPtt();
    h.windowNode.dispatch('pagehide', {});
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases on Escape while keyed', async () => {
    const h = txReleaseHarness();
    await h.api.setPtt(true, { useMic: false });
    await h.flush();
    expect(h.state.moxRequested).toBe(true);
    h.documentNode.dispatch('keydown', { key: 'Escape' });
    await h.flush();
    expectIdleAfterRelease(h);
  });

  it('releases a request that is still engaging, before the bridge confirms keyed', async () => {
    const h = txReleaseHarness();
    // In-flight keying request: PTT is held, MOX is requested, but the bridge has
    // not confirmed keyed yet, so the phase is still rx. Releasing must clear the
    // request rather than leave it pending for a late confirmation to key.
    h.state.pointerPttActive = true;
    h.state.pointerPttPointerId = 1;
    h.state.moxRequested = true;
    h.state.txPhase = 'rx';
    h.element('ptt-btn').dispatch('pointerup', { pointerId: 1, pointerType: 'touch' });
    await h.flush();
    expect(h.state.moxRequested).toBe(false);
    expectIdleAfterRelease(h);
  });

  it('keeps the socket-close release calls in the connection handler', () => {
    const onclose = slice('      ws.onclose = (event) => {', '      ws.onerror = (event) => {');
    expect(onclose).toContain('recordOperatorFault("Link lost during TX", "alarm", false)');
    expect(onclose).toContain('lockTx("reconnect", false)');
    expect(onclose).toContain('stopMicCapture()');
    expect(onclose).toContain('stopRxAudio(false, false)');
  });
});
