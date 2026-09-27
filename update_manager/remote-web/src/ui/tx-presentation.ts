export type TxControlPresentationState =
  | 'disabled'
  | 'fault'
  | 'locked'
  | 'armed'
  | 'engaging'
  | 'transmitting';

export interface TxControlPresentationInput {
  connected: boolean;
  blocked: boolean;
  receiveOnly: boolean;
  faulted: boolean;
  ready: boolean;
  txPhase: string;
  requested: boolean;
  enabled: boolean;
}

export function txControlPresentationState(
  input: TxControlPresentationInput,
): TxControlPresentationState {
  if (input.txPhase === 'keyed') return 'transmitting';
  if (input.txPhase === 'armed' || input.requested || input.enabled) return 'engaging';
  if (input.receiveOnly || input.blocked || !input.connected) return 'disabled';
  if (input.faulted) return 'fault';
  if (input.ready) return 'armed';
  return 'locked';
}

export interface TxActionAvailability {
  arm: boolean;
  ptt: boolean;
  mox: boolean;
  lock: boolean;
}

export function txActionAvailability(
  state: TxControlPresentationState,
  activeSource: string,
): TxActionAvailability {
  const source = `${activeSource || ''}`.trim().toLowerCase();
  const active = state === 'engaging' || state === 'transmitting';
  return {
    arm: state === 'locked' || state === 'fault',
    ptt: state === 'armed' || (active && source === 'ptt'),
    mox: state === 'armed' || (active && source === 'mox'),
    lock: state !== 'disabled',
  };
}

/**
 * The build sheet's five visual states (Locked / Disarmed / Armed / Transmitting / Fault),
 * mapped by behavior and reason rather than by {@link TxControlPresentationState} name.
 * `engaging` is kept distinct from `armed` (same color, different text) rather than folded
 * into it, so callers that need the five-state set collapse it themselves.
 */
export type TxVisualState = 'locked' | 'disarmed' | 'armed' | 'engaging' | 'transmitting' | 'fault';

/**
 * Code `disabled` is always visual Locked. Code `locked` splits on *why*: an explicit
 * `operator-lock` reason is still visual Locked, but everything else that lands in code
 * `locked` (never armed yet, or an expired `idle-timeout` lease) is visual Disarmed with a
 * reconfirmation hint, per decisions.md "TX visual states".
 */
export function txVisualState(
  presentationState: TxControlPresentationState,
  txLockReason: string,
): TxVisualState {
  if (presentationState === 'disabled') return 'locked';
  if (presentationState === 'locked') {
    return txLockReason === 'operator-lock' ? 'locked' : 'disarmed';
  }
  return presentationState;
}

export const TX_VISUAL_STATE_BADGE_TEXT: Readonly<Record<TxVisualState, string>> = {
  locked: 'TX locked',
  disarmed: 'TX disarmed',
  armed: 'TX armed',
  engaging: 'Keying…',
  transmitting: 'On air',
  fault: 'TX fault',
};

export const TX_VISUAL_STATE_ANNOUNCEMENT: Readonly<Record<TxVisualState, string>> = {
  locked: 'TX locked',
  disarmed: 'TX disarmed, reconfirm to arm',
  armed: 'TX armed',
  engaging: 'TX keying, request sent',
  transmitting: 'TX on air',
  fault: 'TX fault, transmit stopped',
};
