import { describe, expect, it, vi } from 'vitest';
import { createLegacySocketAdapter } from '../src/transport/legacy-socket-adapter';
import type {
  SplitSocketEvent,
  SplitSocketEventType,
  SplitSocketLike,
} from '../src/transport/split-sockets';
import { SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES } from '../src/transport/split-sockets';

class FakeWebSocket implements SplitSocketLike {
  readonly sent: Array<string | ArrayBuffer> = [];
  readonly listeners = new Map<SplitSocketEventType, Array<(event: SplitSocketEvent) => void>>();
  readyState = 0;
  bufferedAmount = 0;
  binaryType?: BinaryType;

  constructor(readonly url: string) {}

  addEventListener(type: SplitSocketEventType, listener: (event: SplitSocketEvent) => void): void {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  close(): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.dispatch('close', {});
  }

  open(): void {
    this.readyState = 1;
    this.dispatch('open', {});
  }

  send(data: string | ArrayBuffer): void {
    this.sent.push(data);
  }

  receive(data: unknown): void {
    this.dispatch('message', { data });
  }

  error(): void {
    this.dispatch('error', {});
  }

  private dispatch(type: SplitSocketEventType, event: SplitSocketEvent): void {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}

function createFakeCtor() {
  const sockets: FakeWebSocket[] = [];
  const WebSocketCtor = class extends FakeWebSocket {
    constructor(url: string) {
      super(url);
      sockets.push(this);
    }
  };
  return { sockets, WebSocketCtor };
}

describe('Phase 42 legacy socket adapter', () => {
  it('opens as a legacy socket only after both lanes are open', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const onopen = vi.fn();
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    adapter.onopen = onopen;
    const [control, media] = sockets;

    expect(adapter.readyState).toBe(0);
    expect(control?.url).toBe('wss://radio.local:8443/saturn/control?session=session-123');
    expect(media?.url).toBe('wss://radio.local:8443/saturn/media?session=session-123');
    expect(media?.binaryType).toBe('arraybuffer');

    control?.open();
    expect(onopen).not.toHaveBeenCalled();
    expect(adapter.readyState).toBe(0);
    expect(control?.sent).toEqual(['session_open:session-123,operator;']);

    media?.open();
    expect(adapter.readyState).toBe(1);
    expect(onopen).toHaveBeenCalledTimes(1);
  });

  it('routes legacy text sends to control and binary sends to media', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    const [control, media] = sockets;
    const frame = new ArrayBuffer(64);
    control?.open();
    media?.open();
    if (media) media.bufferedAmount = 512;

    adapter.send('trx:0,false;');
    adapter.send(frame);

    expect(adapter.bufferedAmount).toBe(512);
    expect(control?.sent).toEqual(['session_open:session-123,operator;', 'trx:0,false;']);
    expect(media?.sent).toEqual([frame]);
  });

  it('leaves media backpressure drops to the legacy caller', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    const [, media] = sockets;
    const frame = new ArrayBuffer(64);
    media?.open();
    if (media) media.bufferedAmount = SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES + 1;

    adapter.send(frame);

    expect(adapter.bufferedAmount).toBe(SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES + 1);
    expect(media?.sent).toEqual([frame]);
  });

  it('dispatches control text and media binary as legacy messages', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const received: unknown[] = [];
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    adapter.onmessage = (event) => received.push(event.data);
    const [control, media] = sockets;
    const frame = new ArrayBuffer(64);

    control?.open();
    media?.open();
    control?.receive('ready;');
    media?.receive(frame);

    expect(received).toEqual(['ready;', frame]);
  });

  it('buffers lane messages until the legacy open event has fired', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const events: string[] = [];
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    adapter.onopen = () => events.push('open');
    adapter.onmessage = (event) => events.push(`message:${String(event.data)}`);
    const [control, media] = sockets;

    control?.open();
    control?.receive('remote_client_role:0,operator,1;');

    expect(events).toEqual([]);

    media?.open();

    expect(events).toEqual(['open', 'message:remote_client_role:0,operator,1;']);
  });

  it('closes the paired lane and emits one legacy close', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const onclose = vi.fn();
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
    });
    adapter.onclose = onclose;
    const [control, media] = sockets;
    control?.open();
    media?.open();

    control?.close();

    expect(media?.readyState).toBe(3);
    expect(adapter.readyState).toBe(3);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it('reports protocol violations without delivering cross-lane payloads', () => {
    const { sockets, WebSocketCtor } = createFakeCtor();
    const onProtocolViolation = vi.fn();
    const onmessage = vi.fn();
    const adapter = createLegacySocketAdapter({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      WebSocketCtor,
      onProtocolViolation,
    });
    adapter.onmessage = onmessage;
    const [control, media] = sockets;

    control?.receive(new ArrayBuffer(4));
    media?.receive('wrong-lane;');

    expect(onmessage).not.toHaveBeenCalled();
    expect(onProtocolViolation).toHaveBeenCalledTimes(2);
  });
});
