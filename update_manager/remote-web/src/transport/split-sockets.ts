export const SPLIT_CONTROL_PATH = '/saturn/control';
export const SPLIT_MEDIA_PATH = '/saturn/media';
export const SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES = 20_480;
export const SPLIT_SOCKET_OPEN = 1;

export type SplitSocketRole = 'control' | 'media';
export type SplitSessionRole = 'operator' | 'viewer';

export type SplitSocketUrls = {
  controlUrl: string;
  mediaUrl: string;
};

export type SplitMediaSendDecision = {
  action: 'send' | 'drop';
  degraded: boolean;
  bufferedBytes: number;
  hardCapBytes: number;
};

export type SplitSocketEventType = 'open' | 'message' | 'close' | 'error';

export type SplitSocketEvent = {
  data?: unknown;
};

export type SplitSocketLike = {
  readyState: number;
  bufferedAmount?: number;
  binaryType?: BinaryType;
  addEventListener(type: SplitSocketEventType, listener: (event: SplitSocketEvent) => void): void;
  close(): void;
  send(data: string | ArrayBuffer): void;
};

export type SplitSocketFactory = (url: string, role: SplitSocketRole) => SplitSocketLike;

export type SplitProtocolViolation = {
  channel: SplitSocketRole;
  dataKind: 'text' | 'binary' | 'other';
  reason: string;
};

export type SplitSocketClientOptions = {
  baseWsUrl: string;
  sessionId: string;
  socketFactory: SplitSocketFactory;
  role?: SplitSessionRole;
  onControlText?: (text: string) => void;
  onMediaBinary?: (buffer: ArrayBuffer) => void;
  onProtocolViolation?: (violation: SplitProtocolViolation) => void;
};

export type SplitControlSendResult = {
  action: 'send' | 'drop';
  reason: 'sent' | 'not-open';
};

export type SplitMediaSendResult = SplitMediaSendDecision & {
  sent: boolean;
  reason: 'sent' | 'backlog' | 'not-open';
};

export type SplitSocketClient = {
  sessionId: string;
  urls: SplitSocketUrls;
  controlSocket: SplitSocketLike;
  mediaSocket: SplitSocketLike;
  sendControl(text: string): SplitControlSendResult;
  sendMedia(frame: ArrayBuffer): SplitMediaSendResult;
  close(): void;
};

export function deriveSplitSocketUrls(baseWsUrl: string, sessionId: string): SplitSocketUrls {
  const trimmedSession = sessionId.trim();
  if (!trimmedSession) {
    throw new Error('split split socket session id is required');
  }

  const control = new URL(baseWsUrl);
  const media = new URL(baseWsUrl);
  control.pathname = SPLIT_CONTROL_PATH;
  media.pathname = SPLIT_MEDIA_PATH;
  control.search = '';
  media.search = '';
  control.searchParams.set('session', trimmedSession);
  media.searchParams.set('session', trimmedSession);
  control.hash = '';
  media.hash = '';

  return {
    controlUrl: control.toString(),
    mediaUrl: media.toString(),
  };
}

export function buildSplitSessionOpenMessage(
  sessionId: string,
  role: SplitSessionRole = 'operator',
): string {
  const trimmedSession = sessionId.trim();
  if (!trimmedSession) {
    throw new Error('split split socket session id is required');
  }
  if (/[,;]/.test(trimmedSession)) {
    throw new Error('split split socket session id cannot contain TCI delimiters');
  }
  return `session_open:${trimmedSession},${role};`;
}

export function decideSplitMediaSend(
  bufferedAmountBytes: number,
  hardCapBytes = SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES,
): SplitMediaSendDecision {
  const bufferedBytes = Math.max(
    0,
    Math.round(Number.isFinite(bufferedAmountBytes) ? bufferedAmountBytes : 0),
  );
  const cap = Math.max(
    0,
    Math.round(Number.isFinite(hardCapBytes) ? hardCapBytes : SPLIT_MEDIA_BACKLOG_HARD_CAP_BYTES),
  );
  const overCap = bufferedBytes > cap;
  return {
    action: overCap ? 'drop' : 'send',
    degraded: overCap,
    bufferedBytes,
    hardCapBytes: cap,
  };
}

function splitDataKind(data: unknown): SplitProtocolViolation['dataKind'] {
  if (typeof data === 'string') return 'text';
  if (data instanceof ArrayBuffer) return 'binary';
  return 'other';
}

function reportSplitProtocolViolation(
  onProtocolViolation: ((violation: SplitProtocolViolation) => void) | undefined,
  channel: SplitSocketRole,
  data: unknown,
  reason: string,
) {
  onProtocolViolation?.({
    channel,
    dataKind: splitDataKind(data),
    reason,
  });
}

export function createSplitSocketClient(
  options: SplitSocketClientOptions,
): SplitSocketClient {
  const sessionId = options.sessionId.trim();
  const urls = deriveSplitSocketUrls(options.baseWsUrl, sessionId);
  const controlSocket = options.socketFactory(urls.controlUrl, 'control');
  const mediaSocket = options.socketFactory(urls.mediaUrl, 'media');
  mediaSocket.binaryType = 'arraybuffer';

  controlSocket.addEventListener('open', () => {
    controlSocket.send(buildSplitSessionOpenMessage(sessionId, options.role ?? 'operator'));
  });
  controlSocket.addEventListener('message', (event) => {
    if (typeof event.data === 'string') {
      options.onControlText?.(event.data);
      return;
    }
    reportSplitProtocolViolation(
      options.onProtocolViolation,
      'control',
      event.data,
      'control socket received non-text payload',
    );
  });
  mediaSocket.addEventListener('message', (event) => {
    if (event.data instanceof ArrayBuffer) {
      options.onMediaBinary?.(event.data);
      return;
    }
    reportSplitProtocolViolation(
      options.onProtocolViolation,
      'media',
      event.data,
      'media socket received non-binary payload',
    );
  });

  return {
    sessionId,
    urls,
    controlSocket,
    mediaSocket,
    sendControl(text) {
      if (controlSocket.readyState !== SPLIT_SOCKET_OPEN) {
        return { action: 'drop', reason: 'not-open' };
      }
      controlSocket.send(text);
      return { action: 'send', reason: 'sent' };
    },
    sendMedia(frame) {
      const decision = decideSplitMediaSend(mediaSocket.bufferedAmount ?? 0);
      if (decision.action === 'drop') {
        return { ...decision, sent: false, reason: 'backlog' };
      }
      if (mediaSocket.readyState !== SPLIT_SOCKET_OPEN) {
        return { ...decision, action: 'drop', sent: false, reason: 'not-open' };
      }
      mediaSocket.send(frame);
      return { ...decision, sent: true, reason: 'sent' };
    },
    close() {
      controlSocket.close();
      mediaSocket.close();
    },
  };
}
