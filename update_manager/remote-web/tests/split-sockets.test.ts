import { describe, expect, it, vi } from 'vitest';
import {
  PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES,
  type Phase42ProtocolViolation,
  type Phase42SocketEvent,
  type Phase42SocketEventType,
  type Phase42SocketLike,
  type Phase42SocketRole,
  buildPhase42SessionOpenMessage,
  createPhase42SplitSocketClient,
  decidePhase42MediaSend,
  derivePhase42SplitSocketUrls,
} from '../src/transport/split-sockets';

class FakeSocket implements Phase42SocketLike {
  readonly sent: Array<string | ArrayBuffer> = [];
  readonly listeners = new Map<Phase42SocketEventType, Array<(event: Phase42SocketEvent) => void>>();
  readyState = 0;
  bufferedAmount = 0;
  binaryType?: BinaryType;

  addEventListener(type: Phase42SocketEventType, listener: (event: Phase42SocketEvent) => void): void {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  close(): void {
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

  private dispatch(type: Phase42SocketEventType, event: Phase42SocketEvent): void {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}

function createFakePair() {
  const sockets = {
    control: new FakeSocket(),
    media: new FakeSocket(),
  };
  const urls: Record<Phase42SocketRole, string> = {
    control: '',
    media: '',
  };
  return {
    sockets,
    urls,
    factory(url: string, role: Phase42SocketRole): FakeSocket {
      urls[role] = url;
      return sockets[role];
    },
  };
}

describe('Phase 42 split socket transport helpers', () => {
  it('derives independent control and media URLs from the legacy TCI URL', () => {
    const urls = derivePhase42SplitSocketUrls('wss://radio.local:8443/tci?old=1', 'session-123');

    expect(urls.controlUrl).toBe('wss://radio.local:8443/saturn/control?session=session-123');
    expect(urls.mediaUrl).toBe('wss://radio.local:8443/saturn/media?session=session-123');
  });

  it('encodes the session id and refuses empty ids', () => {
    const urls = derivePhase42SplitSocketUrls('ws://127.0.0.1:50001/tci', 'phase 42');

    expect(urls.controlUrl).toBe('ws://127.0.0.1:50001/saturn/control?session=phase+42');
    expect(urls.mediaUrl).toBe('ws://127.0.0.1:50001/saturn/media?session=phase+42');
    expect(() => derivePhase42SplitSocketUrls('ws://127.0.0.1:50001/tci', '  ')).toThrow(
      /session id is required/,
    );
  });

  it('builds the control-plane session open message', () => {
    expect(buildPhase42SessionOpenMessage('session-123')).toBe('session_open:session-123,operator;');
    expect(buildPhase42SessionOpenMessage('session-123', 'viewer')).toBe('session_open:session-123,viewer;');
    expect(() => buildPhase42SessionOpenMessage('bad,session')).toThrow(/TCI delimiters/);
  });

  it('sends below the media backlog cap', () => {
    const decision = decidePhase42MediaSend(PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES);

    expect(decision.action).toBe('send');
    expect(decision.degraded).toBe(false);
    expect(decision.hardCapBytes).toBe(PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES);
  });

  it('drops before committing media bytes above the cap', () => {
    const send = vi.fn();
    const decision = decidePhase42MediaSend(PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES + 1);

    if (decision.action === 'send') {
      send(new ArrayBuffer(64));
    }

    expect(decision.action).toBe('drop');
    expect(decision.degraded).toBe(true);
    expect(send).not.toHaveBeenCalled();
  });

  it('opens paired sockets and announces the session on the control lane', () => {
    const pair = createFakePair();
    const client = createPhase42SplitSocketClient({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      socketFactory: pair.factory,
    });

    expect(client.urls.controlUrl).toBe('wss://radio.local:8443/saturn/control?session=session-123');
    expect(client.urls.mediaUrl).toBe('wss://radio.local:8443/saturn/media?session=session-123');
    expect(pair.urls.control).toBe(client.urls.controlUrl);
    expect(pair.urls.media).toBe(client.urls.mediaUrl);
    expect(pair.sockets.media.binaryType).toBe('arraybuffer');

    pair.sockets.control.open();

    expect(pair.sockets.control.sent).toEqual(['session_open:session-123,operator;']);
    expect(pair.sockets.media.sent).toEqual([]);
  });

  it('routes control text and media binary while reporting lane violations', () => {
    const pair = createFakePair();
    const controlText: string[] = [];
    const mediaFrames: ArrayBuffer[] = [];
    const violations: Phase42ProtocolViolation[] = [];
    createPhase42SplitSocketClient({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      socketFactory: pair.factory,
      onControlText: (text) => controlText.push(text),
      onMediaBinary: (buffer) => mediaFrames.push(buffer),
      onProtocolViolation: (violation) => violations.push(violation),
    });
    const mediaFrame = new ArrayBuffer(64);

    pair.sockets.control.receive('session_paired:session-123;');
    pair.sockets.media.receive(mediaFrame);
    pair.sockets.control.receive(new ArrayBuffer(4));
    pair.sockets.media.receive('ready;');

    expect(controlText).toEqual(['session_paired:session-123;']);
    expect(mediaFrames).toEqual([mediaFrame]);
    expect(violations).toEqual([
      {
        channel: 'control',
        dataKind: 'binary',
        reason: 'control socket received non-text payload',
      },
      {
        channel: 'media',
        dataKind: 'text',
        reason: 'media socket received non-binary payload',
      },
    ]);
  });

  it('sends control text and media frames on separate open sockets', () => {
    const pair = createFakePair();
    const client = createPhase42SplitSocketClient({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      socketFactory: pair.factory,
    });
    const mediaFrame = new ArrayBuffer(64);
    pair.sockets.control.open();
    pair.sockets.media.open();

    expect(client.sendControl('trx:0,false;')).toEqual({ action: 'send', reason: 'sent' });
    expect(client.sendMedia(mediaFrame)).toEqual({
      action: 'send',
      degraded: false,
      bufferedBytes: 0,
      hardCapBytes: PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES,
      sent: true,
      reason: 'sent',
    });

    expect(pair.sockets.control.sent).toEqual(['session_open:session-123,operator;', 'trx:0,false;']);
    expect(pair.sockets.media.sent).toEqual([mediaFrame]);
  });

  it('drops media frames above the hard cap before committing bytes', () => {
    const pair = createFakePair();
    const client = createPhase42SplitSocketClient({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      socketFactory: pair.factory,
    });
    pair.sockets.media.open();
    pair.sockets.media.bufferedAmount = PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES + 1;

    const decision = client.sendMedia(new ArrayBuffer(64));

    expect(decision.action).toBe('drop');
    expect(decision.sent).toBe(false);
    expect(decision.reason).toBe('backlog');
    expect(pair.sockets.media.sent).toEqual([]);
  });
});
