import {
  SPLIT_SOCKET_OPEN,
  type SplitProtocolViolation,
  type SplitSessionRole,
  type SplitSocketEvent,
  type SplitSocketEventType,
  type SplitSocketLike,
  type SplitSocketRole,
  type SplitSocketClientOptions,
  type SplitSocketClient,
  type SplitSocketUrls,
  createSplitSocketClient,
} from './split-sockets';

const SOCKET_CONNECTING = 0;
const SOCKET_CLOSING = 2;
const SOCKET_CLOSED = 3;

type LegacyAdapterEventHandler = (event: SplitSocketEvent) => void;
type SplitWebSocketCtor = new (url: string) => SplitSocketLike;

export type LegacySocketAdapterOptions = {
  baseWsUrl: string;
  sessionId: string;
  WebSocketCtor: SplitWebSocketCtor;
  role?: SplitSessionRole;
  onProtocolViolation?: (violation: SplitProtocolViolation) => void;
};

export type LegacySocketAdapter = SplitSocketLike & {
  readonly client: SplitSocketClient;
  readonly urls: SplitSocketUrls;
  onopen: LegacyAdapterEventHandler | null;
  onmessage: LegacyAdapterEventHandler | null;
  onclose: LegacyAdapterEventHandler | null;
  onerror: LegacyAdapterEventHandler | null;
  removeEventListener(type: SplitSocketEventType, listener: LegacyAdapterEventHandler): void;
};

export function createLegacySocketAdapter(
  options: LegacySocketAdapterOptions,
): LegacySocketAdapter {
  const sockets: Partial<Record<SplitSocketRole, SplitSocketLike>> = {};
  const listeners: { [K in SplitSocketEventType]: Set<LegacyAdapterEventHandler> } = {
    open: new Set(),
    message: new Set(),
    close: new Set(),
    error: new Set(),
  };
  let adapterBinaryType: BinaryType = 'arraybuffer';
  let onopen: LegacyAdapterEventHandler | null = null;
  let onmessage: LegacyAdapterEventHandler | null = null;
  let onclose: LegacyAdapterEventHandler | null = null;
  let onerror: LegacyAdapterEventHandler | null = null;
  let openEmitted = false;
  let closeRequested = false;
  let closeEmitted = false;
  const preOpenMessages: SplitSocketEvent[] = [];

  function dispatch(type: SplitSocketEventType, event: SplitSocketEvent = {}): void {
    const handler =
      type === 'open' ? onopen :
      type === 'message' ? onmessage :
      type === 'close' ? onclose :
      onerror;
    handler?.(event);
    for (const listener of listeners[type]) {
      listener(event);
    }
  }

  function closeSocket(socket: SplitSocketLike | undefined): void {
    if (socket && socket.readyState < SOCKET_CLOSING) {
      socket.close();
    }
  }

  function readyStateForPair(): number {
    if (closeEmitted) return SOCKET_CLOSED;
    if (closeRequested) return SOCKET_CLOSING;
    const controlState = sockets.control?.readyState;
    const mediaState = sockets.media?.readyState;
    if (controlState === SPLIT_SOCKET_OPEN && mediaState === SPLIT_SOCKET_OPEN) {
      return SPLIT_SOCKET_OPEN;
    }
    if (controlState === SOCKET_CLOSED || mediaState === SOCKET_CLOSED) {
      return SOCKET_CLOSED;
    }
    if (controlState === SOCKET_CLOSING || mediaState === SOCKET_CLOSING) {
      return SOCKET_CLOSING;
    }
    return SOCKET_CONNECTING;
  }

  function maybeDispatchOpen(event: SplitSocketEvent): void {
    if (
      !openEmitted &&
      !closeRequested &&
      sockets.control?.readyState === SPLIT_SOCKET_OPEN &&
      sockets.media?.readyState === SPLIT_SOCKET_OPEN
    ) {
      openEmitted = true;
      dispatch('open', event);
      while (preOpenMessages.length > 0 && !closeRequested) {
        const message = preOpenMessages.shift();
        if (message) dispatch('message', message);
      }
    }
  }

  function dispatchMessage(event: SplitSocketEvent): void {
    if (!openEmitted) {
      preOpenMessages.push(event);
      return;
    }
    dispatch('message', event);
  }

  function dispatchCloseOnce(event: SplitSocketEvent): void {
    if (closeEmitted) return;
    closeRequested = true;
    closeEmitted = true;
    closeSocket(sockets.control);
    closeSocket(sockets.media);
    dispatch('close', event);
  }

  function socketFactory(url: string, role: SplitSocketRole): SplitSocketLike {
    const socket = new options.WebSocketCtor(url);
    sockets[role] = socket;
    if (role === 'media') {
      socket.binaryType = adapterBinaryType;
    }
    socket.addEventListener('open', (event) => maybeDispatchOpen(event));
    socket.addEventListener('close', (event) => dispatchCloseOnce(event));
    socket.addEventListener('error', (event) => dispatch('error', event));
    return socket;
  }

  const splitOptions: SplitSocketClientOptions = {
    baseWsUrl: options.baseWsUrl,
    sessionId: options.sessionId,
    socketFactory,
    onControlText: (text) => dispatchMessage({ data: text }),
    onMediaBinary: (buffer) => dispatchMessage({ data: buffer }),
  };
  if (options.role !== undefined) {
    splitOptions.role = options.role;
  }
  if (options.onProtocolViolation !== undefined) {
    splitOptions.onProtocolViolation = options.onProtocolViolation;
  }
  const client = createSplitSocketClient(splitOptions);

  return {
    get client() {
      return client;
    },
    get urls() {
      return client.urls;
    },
    get readyState() {
      return readyStateForPair();
    },
    get bufferedAmount() {
      return Math.max(0, Math.round(sockets.media?.bufferedAmount ?? 0));
    },
    get binaryType() {
      return adapterBinaryType;
    },
    set binaryType(value: BinaryType) {
      adapterBinaryType = value;
      if (sockets.media) {
        sockets.media.binaryType = adapterBinaryType;
      }
    },
    get onopen() {
      return onopen;
    },
    set onopen(handler: LegacyAdapterEventHandler | null) {
      onopen = handler;
    },
    get onmessage() {
      return onmessage;
    },
    set onmessage(handler: LegacyAdapterEventHandler | null) {
      onmessage = handler;
    },
    get onclose() {
      return onclose;
    },
    set onclose(handler: LegacyAdapterEventHandler | null) {
      onclose = handler;
    },
    get onerror() {
      return onerror;
    },
    set onerror(handler: LegacyAdapterEventHandler | null) {
      onerror = handler;
    },
    addEventListener(type, listener) {
      listeners[type].add(listener);
    },
    removeEventListener(type, listener) {
      listeners[type].delete(listener);
    },
    close() {
      if (closeEmitted) return;
      closeRequested = true;
      client.close();
      if (sockets.control?.readyState === SOCKET_CLOSED && sockets.media?.readyState === SOCKET_CLOSED) {
        dispatchCloseOnce({});
      }
    },
    send(data) {
      if (typeof data === 'string') {
        const result = client.sendControl(data);
        if (result.action === 'drop') {
          throw new Error('split control socket is not open');
        }
        return;
      }
      if (data instanceof ArrayBuffer) {
        if (sockets.media?.readyState !== SPLIT_SOCKET_OPEN) {
          throw new Error('split media socket is not open');
        }
        sockets.media.send(data);
        return;
      }
      throw new Error('split legacy socket adapter only supports string and ArrayBuffer payloads');
    },
  };
}
