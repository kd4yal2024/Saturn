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
