export const PHASE42_CONTROL_PATH = '/saturn/control';
export const PHASE42_MEDIA_PATH = '/saturn/media';
export const PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES = 20_480;
export const PHASE42_SOCKET_OPEN = 1;

export type Phase42SocketRole = 'control' | 'media';
export type Phase42SessionRole = 'operator' | 'viewer';

export type SplitSocketUrls = {
  controlUrl: string;
  mediaUrl: string;
};

export type Phase42MediaSendDecision = {
  action: 'send' | 'drop';
  degraded: boolean;
  bufferedBytes: number;
  hardCapBytes: number;
};

export type Phase42SocketEventType = 'open' | 'message' | 'close' | 'error';

export type Phase42SocketEvent = {
  data?: unknown;
};

export type Phase42SocketLike = {
  readyState: number;
  bufferedAmount?: number;
  binaryType?: BinaryType;
  addEventListener(type: Phase42SocketEventType, listener: (event: Phase42SocketEvent) => void): void;
  close(): void;
  send(data: string | ArrayBuffer): void;
};

export type Phase42SocketFactory = (url: string, role: Phase42SocketRole) => Phase42SocketLike;

export type Phase42ProtocolViolation = {
  channel: Phase42SocketRole;
  dataKind: 'text' | 'binary' | 'other';
  reason: string;
};

export type Phase42SplitSocketClientOptions = {
  baseWsUrl: string;
  sessionId: string;
  socketFactory: Phase42SocketFactory;
  role?: Phase42SessionRole;
  onControlText?: (text: string) => void;
  onMediaBinary?: (buffer: ArrayBuffer) => void;
  onProtocolViolation?: (violation: Phase42ProtocolViolation) => void;
};

export type Phase42ControlSendResult = {
  action: 'send' | 'drop';
  reason: 'sent' | 'not-open';
};

export type Phase42MediaSendResult = Phase42MediaSendDecision & {
  sent: boolean;
  reason: 'sent' | 'backlog' | 'not-open';
};

export type Phase42SplitSocketClient = {
  sessionId: string;
  urls: SplitSocketUrls;
  controlSocket: Phase42SocketLike;
  mediaSocket: Phase42SocketLike;
  sendControl(text: string): Phase42ControlSendResult;
  sendMedia(frame: ArrayBuffer): Phase42MediaSendResult;
  close(): void;
};

export function derivePhase42SplitSocketUrls(baseWsUrl: string, sessionId: string): SplitSocketUrls {
  const trimmedSession = sessionId.trim();
  if (!trimmedSession) {
    throw new Error('phase42 split socket session id is required');
  }

  const control = new URL(baseWsUrl);
  const media = new URL(baseWsUrl);
  control.pathname = PHASE42_CONTROL_PATH;
  media.pathname = PHASE42_MEDIA_PATH;
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

export function buildPhase42SessionOpenMessage(
  sessionId: string,
  role: Phase42SessionRole = 'operator',
): string {
  const trimmedSession = sessionId.trim();
  if (!trimmedSession) {
    throw new Error('phase42 split socket session id is required');
  }
  if (/[,;]/.test(trimmedSession)) {
    throw new Error('phase42 split socket session id cannot contain TCI delimiters');
  }
  return `session_open:${trimmedSession},${role};`;
}

export function decidePhase42MediaSend(
  bufferedAmountBytes: number,
  hardCapBytes = PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES,
): Phase42MediaSendDecision {
  const bufferedBytes = Math.max(
    0,
    Math.round(Number.isFinite(bufferedAmountBytes) ? bufferedAmountBytes : 0),
  );
  const cap = Math.max(
    0,
    Math.round(Number.isFinite(hardCapBytes) ? hardCapBytes : PHASE42_MEDIA_BACKLOG_HARD_CAP_BYTES),
  );
  const overCap = bufferedBytes > cap;
  return {
    action: overCap ? 'drop' : 'send',
    degraded: overCap,
    bufferedBytes,
    hardCapBytes: cap,
  };
}

function phase42DataKind(data: unknown): Phase42ProtocolViolation['dataKind'] {
  if (typeof data === 'string') return 'text';
  if (data instanceof ArrayBuffer) return 'binary';
  return 'other';
}

function reportPhase42ProtocolViolation(
  onProtocolViolation: ((violation: Phase42ProtocolViolation) => void) | undefined,
  channel: Phase42SocketRole,
  data: unknown,
  reason: string,
) {
  onProtocolViolation?.({
    channel,
    dataKind: phase42DataKind(data),
    reason,
  });
}

export function createPhase42SplitSocketClient(
  options: Phase42SplitSocketClientOptions,
): Phase42SplitSocketClient {
  const sessionId = options.sessionId.trim();
  const urls = derivePhase42SplitSocketUrls(options.baseWsUrl, sessionId);
  const controlSocket = options.socketFactory(urls.controlUrl, 'control');
  const mediaSocket = options.socketFactory(urls.mediaUrl, 'media');
  mediaSocket.binaryType = 'arraybuffer';

  controlSocket.addEventListener('open', () => {
    controlSocket.send(buildPhase42SessionOpenMessage(sessionId, options.role ?? 'operator'));
  });
  controlSocket.addEventListener('message', (event) => {
    if (typeof event.data === 'string') {
      options.onControlText?.(event.data);
      return;
    }
    reportPhase42ProtocolViolation(
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
    reportPhase42ProtocolViolation(
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
      if (controlSocket.readyState !== PHASE42_SOCKET_OPEN) {
        return { action: 'drop', reason: 'not-open' };
      }
      controlSocket.send(text);
      return { action: 'send', reason: 'sent' };
    },
    sendMedia(frame) {
      const decision = decidePhase42MediaSend(mediaSocket.bufferedAmount ?? 0);
      if (decision.action === 'drop') {
        return { ...decision, sent: false, reason: 'backlog' };
      }
      if (mediaSocket.readyState !== PHASE42_SOCKET_OPEN) {
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
