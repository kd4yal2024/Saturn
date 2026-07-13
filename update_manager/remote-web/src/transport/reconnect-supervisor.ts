export type ReconnectPhase =
  | 'idle'
  | 'waiting'
  | 'connecting'
  | 'awaiting-ready'
  | 'online'
  | 'offline';

export type ReconnectSnapshot = {
  active: boolean;
  phase: ReconnectPhase;
  attempt: number;
  nextRetryAt: number | null;
  lastReason: string;
  generation: number;
};

type TimerHandle = ReturnType<typeof setTimeout>;

export type ReconnectSupervisorOptions = {
  connect: (generation: number) => void;
  closeSocket: (reason: string) => void;
  onChange?: (snapshot: ReconnectSnapshot) => void;
  now?: () => number;
  random?: () => number;
  setTimer?: (callback: () => void, delayMs: number) => TimerHandle;
  clearTimer?: (handle: TimerHandle) => void;
  retryDelaysMs?: readonly number[];
  jitterRatio?: number;
  connectTimeoutMs?: number;
  readyTimeoutMs?: number;
};

export type ReconnectSupervisor = {
  start(): void;
  stop(reason?: string): void;
  socketOpened(generation: number): boolean;
  bridgeReady(generation: number): boolean;
  socketClosed(reason?: string): void;
  attemptFailed(reason: string): void;
  setOnline(online: boolean): void;
  retryNow(reason?: string): void;
  snapshot(): ReconnectSnapshot;
  dispose(): void;
};

const DEFAULT_RETRY_DELAYS_MS = [1000, 2000, 4000, 8000, 15000, 30000] as const;

function positiveMilliseconds(value: number | undefined, fallback: number): number {
  return Number.isFinite(value) && Number(value) > 0 ? Number(value) : fallback;
}

function boundedJitterRatio(value: number | undefined): number {
  if (!Number.isFinite(value)) return 0.2;
  return Math.max(0, Math.min(0.5, Number(value)));
}

export function reconnectDelayMs(
  failedAttempt: number,
  randomValue = 0.5,
  retryDelaysMs: readonly number[] = DEFAULT_RETRY_DELAYS_MS,
  jitterRatio = 0.2,
): number {
  const delays = retryDelaysMs.length > 0 ? retryDelaysMs : DEFAULT_RETRY_DELAYS_MS;
  const index = Math.max(0, Math.min(delays.length - 1, Math.round(failedAttempt) - 1));
  const base = positiveMilliseconds(delays[index], DEFAULT_RETRY_DELAYS_MS[index] ?? 1000);
  const ratio = boundedJitterRatio(jitterRatio);
  const random = Math.max(0, Math.min(1, Number.isFinite(randomValue) ? randomValue : 0.5));
  const multiplier = 1 + ((random * 2) - 1) * ratio;
  return Math.max(0, Math.round(base * multiplier));
}

export function createReconnectSupervisor(options: ReconnectSupervisorOptions): ReconnectSupervisor {
  const now = options.now ?? (() => Date.now());
  const random = options.random ?? Math.random;
  const setTimer = options.setTimer ?? ((callback, delayMs) => setTimeout(callback, delayMs));
  const clearTimer = options.clearTimer ?? ((handle) => clearTimeout(handle));
  const retryDelaysMs = options.retryDelaysMs?.length
    ? options.retryDelaysMs
    : DEFAULT_RETRY_DELAYS_MS;
  const jitterRatio = boundedJitterRatio(options.jitterRatio);
  const connectTimeoutMs = positiveMilliseconds(options.connectTimeoutMs, 10000);
  const readyTimeoutMs = positiveMilliseconds(options.readyTimeoutMs, 10000);

  let active = false;
  let online = true;
  let phase: ReconnectPhase = 'idle';
  let attempt = 0;
  let nextRetryAt: number | null = null;
  let lastReason = '';
  let generation = 0;
  let retryTimer: TimerHandle | null = null;
  let watchdogTimer: TimerHandle | null = null;

  function currentSnapshot(): ReconnectSnapshot {
    return { active, phase, attempt, nextRetryAt, lastReason, generation };
  }

  function publish(): void {
    options.onChange?.(currentSnapshot());
  }

  function clearRetryTimer(): void {
    if (retryTimer !== null) {
      clearTimer(retryTimer);
      retryTimer = null;
    }
    nextRetryAt = null;
  }

  function clearWatchdog(): void {
    if (watchdogTimer !== null) {
      clearTimer(watchdogTimer);
      watchdogTimer = null;
    }
  }

  function scheduleWatchdog(reason: string, timeoutMs: number): void {
    clearWatchdog();
    const watchedGeneration = generation;
    watchdogTimer = setTimer(() => {
      watchdogTimer = null;
      if (!active || watchedGeneration !== generation) return;
      failAttempt(reason, true);
    }, timeoutMs);
  }

  function beginAttempt(): void {
    if (!active) return;
    clearRetryTimer();
    clearWatchdog();
    if (!online) {
      phase = 'offline';
      publish();
      return;
    }
    generation += 1;
    attempt += 1;
    phase = 'connecting';
    lastReason = attempt === 1 && !lastReason ? 'initial connection' : lastReason;
    publish();
    scheduleWatchdog('socket connect timeout', connectTimeoutMs);
    try {
      options.connect(generation);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      failAttempt(`connect failed: ${message}`, false);
    }
  }

  function scheduleRetry(reason: string): void {
    if (!active) return;
    clearRetryTimer();
    clearWatchdog();
    lastReason = reason;
    if (!online) {
      phase = 'offline';
      publish();
      return;
    }
    const delayMs = reconnectDelayMs(attempt, random(), retryDelaysMs, jitterRatio);
    phase = 'waiting';
    nextRetryAt = now() + delayMs;
    publish();
    retryTimer = setTimer(() => {
      retryTimer = null;
      nextRetryAt = null;
      beginAttempt();
    }, delayMs);
  }

  function failAttempt(reason: string, closeSocket: boolean): void {
    if (!active || phase === 'waiting' || phase === 'offline' || phase === 'idle') return;
    scheduleRetry(reason);
    if (closeSocket) {
      options.closeSocket(reason);
    }
  }

  return {
    start() {
      if (active) return;
      active = true;
      attempt = 0;
      lastReason = '';
      beginAttempt();
    },

    stop(reason = 'manual disconnect') {
      active = false;
      generation += 1;
      clearRetryTimer();
      clearWatchdog();
      phase = 'idle';
      attempt = 0;
      lastReason = reason;
      publish();
    },

    socketOpened(socketGeneration) {
      if (!active || socketGeneration !== generation || phase !== 'connecting') return false;
      phase = 'awaiting-ready';
      lastReason = 'socket open; waiting for bridge';
      publish();
      scheduleWatchdog('bridge ready timeout', readyTimeoutMs);
      return true;
    },

    bridgeReady(socketGeneration) {
      if (!active || socketGeneration !== generation || phase !== 'awaiting-ready') return false;
      clearWatchdog();
      phase = 'online';
      attempt = 0;
      nextRetryAt = null;
      lastReason = '';
      publish();
      return true;
    },

    socketClosed(reason = 'socket closed') {
      failAttempt(reason, false);
    },

    attemptFailed(reason) {
      failAttempt(reason, true);
    },

    setOnline(isOnline) {
      if (online === isOnline) return;
      online = isOnline;
      if (!active) return;
      if (!online) {
        clearRetryTimer();
        clearWatchdog();
        phase = 'offline';
        lastReason = 'browser offline';
        publish();
        options.closeSocket('browser offline');
        return;
      }
      lastReason = 'browser online';
      beginAttempt();
    },

    retryNow(reason = 'retry requested') {
      if (!active || !online || phase === 'online' || phase === 'connecting' || phase === 'awaiting-ready') {
        return;
      }
      lastReason = reason;
      beginAttempt();
    },

    snapshot: currentSnapshot,

    dispose() {
      active = false;
      generation += 1;
      clearRetryTimer();
      clearWatchdog();
      phase = 'idle';
    },
  };
}
