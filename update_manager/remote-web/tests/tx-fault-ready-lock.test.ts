import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import { createAppState } from '../src/state/app-state';
import { applyTciText } from '../src/tci/apply';
import type { TciRadioState } from '../src/tci/state';
import { txActionAvailability, txControlPresentationState } from '../src/ui/tx-presentation';

const template = readFileSync(
  new URL('../../templates/saturn-remote-next.html', import.meta.url),
  'utf8',
);

function templateSection(startMarker: string, endMarker: string): string {
  const start = template.indexOf(startMarker);
  const end = template.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error(`Missing template section: ${startMarker}`);
  return template.slice(start, end);
}

function txHarness() {
  const state = createAppState();
  state.connected = true;
  state.ws = { readyState: 1 } as WebSocket;
  state.remoteClientRole = 'operator';
  state.bridgeReady = true;
  state.audioStreaming = true;
  state.txPhase = 'keyed';
  state.txEnabled = true;
  state.moxRequested = true;
  state.micCapturing = true;
  state.txReadyExpiresAt = 301_000;
  state.txLockReason = '';

  const commands: string[] = [];
  let micStops = 0;
  let rxRecoveries = 0;
  let rfBlocked = false;
  const context = {
    state,
    performance: { now: () => 1_000 },
    WebSocket: { OPEN: 1 },
    TX_READY_WINDOW_MS: 300_000,
    TX_LOCAL_REQUEST_PERMIT_MS: 5_000,
    satpSourceRequestAt: 0,
    _next: { txUsesBrowserMic: () => false, satpTxBlockReason: () => null },
    clearInterval: () => {},
    setInterval: () => 1,
    rfDisabledBlocksTx: () => rfBlocked,
    clientRoleBlocksTx: () => false,
    sendTci: (command: string) => commands.push(command),
    stopMicCapture: () => { micStops += 1; state.micCapturing = false; },
    recoverRxAfterMox: () => { rxRecoveries += 1; },
    applyTxOpusLabCfcGuard: () => false,
    syncPureSignalOperatorTelemetry: () => {},
    recordOperatorFault: () => {},
    clearOperatorFault: () => {},
    clearTxTimeouts: () => {},
    updateUi: () => {},
    logEvent: () => {},
    refreshBridgeControlHeartbeatForTx: () => {},
    applyTxSourceOverrides: () => {},
    setTxMediaPriority: () => {},
    muteRxAudioForMox: () => {},
    switchDisplayToTxArm: () => {},
    generatedTxSourceActive: () => false,
  };
  const code = [
    templateSection('    function clearTxReadyTimer()', '    function normalizeTxReadyState('),
    templateSection('    function normalizeTxReadyState(', '    function txReadyRemainingMs('),
    templateSection('    async function setPtt(', '    async function setTwoToneEnabled('),
    templateSection('    function syncTciUiSideEffects(', '    function clearTxTimeouts('),
    '({ syncTciUiSideEffects, normalizeTxReadyState, armTxReady, lockTx, setPtt })',
  ].join('\n');
  const controls = runInNewContext(code, context) as {
    syncTciUiSideEffects: (previous: typeof state, next: typeof state, events: ReturnType<typeof applyTciText>) => void;
    normalizeTxReadyState: () => boolean;
    armTxReady: (reason: string) => boolean;
    lockTx: (reason: string, refresh: boolean) => void;
    setPtt: (active: boolean, options?: { useMic?: boolean }) => Promise<boolean>;
  };

  function receive(text: string) {
    const previous = {
      ...state,
      rxEqBands: state.rxEqBands.slice(),
      txEqBands: state.txEqBands.slice(),
      cfcBands: state.cfcBands.slice(),
    };
    const result = applyTciText(text, state as unknown as TciRadioState);
    controls.syncTciUiSideEffects(previous, result.state as typeof state, result);
    return result;
  }

  return {
    state,
    controls,
    commands,
    receive,
    micStops: () => micStops,
    rxRecoveries: () => rxRecoveries,
    setRfBlocked: (blocked: boolean) => { rfBlocked = blocked; },
  };
}

describe('TX fault release readiness', () => {
  it('keeps the fault lock after keyed TX release and requires a fresh arming gesture', async () => {
    const h = txHarness();
    const result = h.receive('tx_fault:0,power_trip,126.3,110.0;');

    expect(result.txFault).toBe('Power trip 126.3 W > 110.0 W');
    expect(result.txReleased).toBe(true);
    expect(h.micStops()).toBe(1);
    expect(h.rxRecoveries()).toBe(1);
    expect(h.state.txPhase).toBe('rx');
    expect(h.state.txLockReason).toBe('tx-fault');
    expect(h.state.txReadyExpiresAt).toBe(0);
    expect(h.controls.normalizeTxReadyState()).toBe(false);

    const presentation = txControlPresentationState({
      connected: h.state.connected,
      blocked: false,
      receiveOnly: false,
      faulted: h.state.txLockReason === 'tx-fault',
      ready: false,
      txPhase: h.state.txPhase,
      requested: h.state.moxRequested,
      enabled: h.state.txEnabled,
    });
    expect(presentation).toBe('fault');
    expect(txActionAvailability(presentation, '')).toMatchObject({ ptt: false, mox: false });

    h.setRfBlocked(true);
    expect(h.controls.normalizeTxReadyState()).toBe(false);
    expect(h.controls.armTxReady('operator-arm')).toBe(false);
    expect(h.state.txLockReason).toBe('tx-fault');
    h.setRfBlocked(false);
    h.controls.lockTx('page-hidden', false);
    expect(h.state.txLockReason).toBe('tx-fault');

    // A PTT press cannot clear a fault; only the separate Arm action can.
    expect(await h.controls.setPtt(true, { useMic: false })).toBe(false);
    expect(h.commands).not.toContain('trx:0,true,tci;');
    expect(h.state.txLockReason).toBe('tx-fault');
    expect(h.state.txReadyExpiresAt).toBe(0);
    expect(h.controls.armTxReady('operator-arm')).toBe(true);
    expect(h.state.txLockReason).toBe('');
    expect(await h.controls.setPtt(true, { useMic: false })).toBe(true);
    expect(h.commands).toContain('trx:0,true,tci;');
  });

  it('still extends readiness after a normal keyed-to-RX release', () => {
    const h = txHarness();
    const result = h.receive('tx_state:0,rx;');

    expect(result.txFault).toBeNull();
    expect(result.txReleased).toBe(true);
    expect(h.micStops()).toBe(1);
    expect(h.rxRecoveries()).toBe(1);
    expect(h.state.txLockReason).toBe('');
    expect(h.state.txReadyExpiresAt).toBe(301_000);
    expect(h.controls.normalizeTxReadyState()).toBe(true);
  });
});
