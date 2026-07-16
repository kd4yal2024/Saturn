import { describe, expect, it, vi } from 'vitest';
import {
  SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES,
  type SplitProtocolViolation,
  type SplitSocketEvent,
  type SplitSocketEventType,
  type SplitSocketLike,
  type SplitSocketRole,
  buildSplitSessionOpenMessage,
  createSplitSocketClient,
  decideSplitMediaSend,
  deriveSplitSocketUrls,
} from '../src/transport/split-sockets';

class FakeSocket implements SplitSocketLike {
  readonly sent: Array<string | ArrayBuffer> = [];
  readonly listeners = new Map<SplitSocketEventType, Array<(event: SplitSocketEvent) => void>>();
  readyState = 0;
  bufferedAmount = 0;
  binaryType?: BinaryType;

  addEventListener(type: SplitSocketEventType, listener: (event: SplitSocketEvent) => void): void {
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

  private dispatch(type: SplitSocketEventType, event: SplitSocketEvent): void {
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
  const urls: Record<SplitSocketRole, string> = {
    control: '',
    media: '',
  };
  return {
    sockets,
    urls,
    factory(url: string, role: SplitSocketRole): FakeSocket {
      urls[role] = url;
      return sockets[role];
    },
  };
}

describe('split socket transport helpers', () => {
  it('derives independent control and media URLs from the legacy TCI URL', () => {
    const urls = deriveSplitSocketUrls('wss://radio.local:8443/tci?old=1', 'session-123');

    expect(urls.controlUrl).toBe('wss://radio.local:8443/saturn/control?session=session-123');
    expect(urls.mediaUrl).toBe('wss://radio.local:8443/saturn/media?session=session-123');
  });

  it('encodes the session id and refuses empty ids', () => {
    const urls = deriveSplitSocketUrls('ws://127.0.0.1:50001/tci', 'phase 42');

    expect(urls.controlUrl).toBe('ws://127.0.0.1:50001/saturn/control?session=phase+42');
    expect(urls.mediaUrl).toBe('ws://127.0.0.1:50001/saturn/media?session=phase+42');
    expect(() => deriveSplitSocketUrls('ws://127.0.0.1:50001/tci', '  ')).toThrow(
      /session id is required/,
    );
  });

  it('builds the control-plane session open message', () => {
    expect(buildSplitSessionOpenMessage('session-123')).toBe('session_open:session-123,operator;');
    expect(buildSplitSessionOpenMessage('session-123', 'viewer')).toBe('session_open:session-123,viewer;');
    expect(() => buildSplitSessionOpenMessage('bad,session')).toThrow(/TCI delimiters/);
  });

  it('sends below the media backlog cap', () => {
    const decision = decideSplitMediaSend(SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES);

    expect(decision.action).toBe('send');
    expect(decision.degraded).toBe(false);
    expect(decision.hardCapBytes).toBe(SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES);
  });

  it('drops before committing media bytes above the cap', () => {
    const send = vi.fn();
    const decision = decideSplitMediaSend(SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES + 1);

    if (decision.action === 'send') {
      send(new ArrayBuffer(64));
    }

    expect(decision.action).toBe('drop');
    expect(decision.degraded).toBe(true);
    expect(send).not.toHaveBeenCalled();
  });

  it('opens paired sockets and announces the session on the control lane', () => {
    const pair = createFakePair();
    const client = createSplitSocketClient({
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
    const violations: SplitProtocolViolation[] = [];
    createSplitSocketClient({
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
    const client = createSplitSocketClient({
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
      hardCapBytes: SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES,
      sent: true,
      reason: 'sent',
    });

    expect(pair.sockets.control.sent).toEqual(['session_open:session-123,operator;', 'trx:0,false;']);
    expect(pair.sockets.media.sent).toEqual([mediaFrame]);
  });

  it('drops media frames above the hard cap before committing bytes', () => {
    const pair = createFakePair();
    const client = createSplitSocketClient({
      baseWsUrl: 'wss://radio.local:8443/tci',
      sessionId: 'session-123',
      socketFactory: pair.factory,
    });
    pair.sockets.media.open();
    pair.sockets.media.bufferedAmount = SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES + 1;

    const decision = client.sendMedia(new ArrayBuffer(64));

    expect(decision.action).toBe('drop');
    expect(decision.sent).toBe(false);
    expect(decision.reason).toBe('backlog');
    expect(pair.sockets.media.sent).toEqual([]);
  });
});
