import { describe, expect, it, vi } from 'vitest';
import {
  createReconnectSupervisor,
  reconnectDelayMs,
  type ReconnectSnapshot,
} from '../src/transport/reconnect-supervisor';

describe('reconnect delay', () => {
  it('backs off and caps at the final delay', () => {
    const delays = [1000, 2000, 4000, 8000];
    expect(reconnectDelayMs(1, 0.5, delays)).toBe(1000);
    expect(reconnectDelayMs(2, 0.5, delays)).toBe(2000);
    expect(reconnectDelayMs(9, 0.5, delays)).toBe(8000);
  });

  it('applies bounded positive and negative jitter', () => {
    expect(reconnectDelayMs(1, 0, [1000], 0.2)).toBe(800);
    expect(reconnectDelayMs(1, 1, [1000], 0.2)).toBe(1200);
  });
});

describe('reconnect supervisor', () => {
  it('connects immediately, waits for ready, and resets backoff after recovery', () => {
    vi.useFakeTimers();
    const connect = vi.fn();
    const closeSocket = vi.fn();
    let snapshot: ReconnectSnapshot | null = null;
    const supervisor = createReconnectSupervisor({
      connect,
      closeSocket,
      onChange: (value) => { snapshot = value; },
      now: () => Date.now(),
      random: () => 0.5,
    });

    supervisor.start();
    expect(connect).toHaveBeenCalledWith(1);
    expect(snapshot?.phase).toBe('connecting');

    expect(supervisor.socketOpened(1)).toBe(true);
    expect(snapshot?.phase).toBe('awaiting-ready');
    expect(supervisor.bridgeReady(1)).toBe(true);
    expect(snapshot?.phase).toBe('online');
    expect(snapshot?.attempt).toBe(0);

    supervisor.socketClosed('bridge restart');
    expect(snapshot?.phase).toBe('waiting');
    expect(snapshot?.nextRetryAt).toBe(Date.now() + 1000);
    vi.advanceTimersByTime(1000);
    expect(connect).toHaveBeenLastCalledWith(2);
    expect(snapshot?.attempt).toBe(1);
    vi.useRealTimers();
  });

  it('closes a socket that never reports bridge readiness and retries once', () => {
    vi.useFakeTimers();
    const connect = vi.fn();
    const closeSocket = vi.fn();
    const supervisor = createReconnectSupervisor({
      connect,
      closeSocket,
      random: () => 0.5,
      readyTimeoutMs: 5000,
    });

    supervisor.start();
    supervisor.socketOpened(1);
    vi.advanceTimersByTime(5000);
    expect(closeSocket).toHaveBeenCalledWith('bridge ready timeout');
    expect(supervisor.snapshot().phase).toBe('waiting');

    supervisor.socketClosed('duplicate close event');
    vi.advanceTimersByTime(1000);
    expect(connect).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it('abandons a WebSocket that remains in CONNECTING', () => {
    vi.useFakeTimers();
    const closeSocket = vi.fn();
    const supervisor = createReconnectSupervisor({
      connect: vi.fn(),
      closeSocket,
      random: () => 0.5,
      connectTimeoutMs: 3000,
    });

    supervisor.start();
    vi.advanceTimersByTime(3000);
    expect(closeSocket).toHaveBeenCalledWith('socket connect timeout');
    expect(supervisor.snapshot().phase).toBe('waiting');
    vi.useRealTimers();
  });

  it('cancels all retry activity after a manual disconnect', () => {
    vi.useFakeTimers();
    const connect = vi.fn();
    const supervisor = createReconnectSupervisor({
      connect,
      closeSocket: vi.fn(),
      random: () => 0.5,
    });

    supervisor.start();
    supervisor.socketClosed('network loss');
    supervisor.stop();
    vi.runAllTimers();

    expect(connect).toHaveBeenCalledTimes(1);
    expect(supervisor.snapshot()).toMatchObject({ active: false, phase: 'idle', attempt: 0 });
    vi.useRealTimers();
  });

  it('pauses while offline and reconnects immediately when the browser returns online', () => {
    vi.useFakeTimers();
    const connect = vi.fn();
    const closeSocket = vi.fn();
    const supervisor = createReconnectSupervisor({ connect, closeSocket, random: () => 0.5 });

    supervisor.start();
    supervisor.setOnline(false);
    expect(supervisor.snapshot().phase).toBe('offline');
    expect(closeSocket).toHaveBeenCalledWith('browser offline');
    vi.runAllTimers();
    expect(connect).toHaveBeenCalledTimes(1);

    supervisor.setOnline(true);
    expect(connect).toHaveBeenCalledTimes(2);
    expect(supervisor.snapshot().phase).toBe('connecting');
    vi.useRealTimers();
  });

  it('ignores stale socket callbacks from an older generation', () => {
    vi.useFakeTimers();
    const supervisor = createReconnectSupervisor({
      connect: vi.fn(),
      closeSocket: vi.fn(),
      random: () => 0.5,
    });

    supervisor.start();
    supervisor.socketClosed('first failure');
    vi.advanceTimersByTime(1000);
    expect(supervisor.snapshot().generation).toBe(2);
    expect(supervisor.socketOpened(1)).toBe(false);
    expect(supervisor.bridgeReady(1)).toBe(false);
    expect(supervisor.snapshot().phase).toBe('connecting');
    vi.useRealTimers();
  });
});
