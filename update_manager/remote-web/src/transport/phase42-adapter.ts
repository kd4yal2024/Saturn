import {
  PHASE42_SOCKET_OPEN,
  type Phase42ProtocolViolation,
  type Phase42SessionRole,
  type Phase42SocketEvent,
  type Phase42SocketEventType,
  type Phase42SocketLike,
  type Phase42SocketRole,
  type Phase42SplitSocketClientOptions,
  type Phase42SplitSocketClient,
  type SplitSocketUrls,
  createPhase42SplitSocketClient,
} from './split-sockets';

const SOCKET_CONNECTING = 0;
const SOCKET_CLOSING = 2;
const SOCKET_CLOSED = 3;

type Phase42AdapterEventHandler = (event: Phase42SocketEvent) => void;
type Phase42WebSocketCtor = new (url: string) => Phase42SocketLike;

export type Phase42LegacySocketAdapterOptions = {
  baseWsUrl: string;
  sessionId: string;
  WebSocketCtor: Phase42WebSocketCtor;
  role?: Phase42SessionRole;
  onProtocolViolation?: (violation: Phase42ProtocolViolation) => void;
};

export type Phase42LegacySocketAdapter = Phase42SocketLike & {
  readonly client: Phase42SplitSocketClient;
  readonly urls: SplitSocketUrls;
  onopen: Phase42AdapterEventHandler | null;
  onmessage: Phase42AdapterEventHandler | null;
  onclose: Phase42AdapterEventHandler | null;
  onerror: Phase42AdapterEventHandler | null;
  removeEventListener(type: Phase42SocketEventType, listener: Phase42AdapterEventHandler): void;
};

export function createPhase42LegacySocketAdapter(
  options: Phase42LegacySocketAdapterOptions,
): Phase42LegacySocketAdapter {
  const sockets: Partial<Record<Phase42SocketRole, Phase42SocketLike>> = {};
  const listeners: { [K in Phase42SocketEventType]: Set<Phase42AdapterEventHandler> } = {
    open: new Set(),
    message: new Set(),
    close: new Set(),
    error: new Set(),
  };
  let adapterBinaryType: BinaryType = 'arraybuffer';
  let onopen: Phase42AdapterEventHandler | null = null;
  let onmessage: Phase42AdapterEventHandler | null = null;
  let onclose: Phase42AdapterEventHandler | null = null;
  let onerror: Phase42AdapterEventHandler | null = null;
  let openEmitted = false;
  let closeRequested = false;
  let closeEmitted = false;
  const preOpenMessages: Phase42SocketEvent[] = [];

  function dispatch(type: Phase42SocketEventType, event: Phase42SocketEvent = {}): void {
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

  function closeSocket(socket: Phase42SocketLike | undefined): void {
    if (socket && socket.readyState < SOCKET_CLOSING) {
      socket.close();
    }
  }

  function readyStateForPair(): number {
    if (closeEmitted) return SOCKET_CLOSED;
    if (closeRequested) return SOCKET_CLOSING;
    const controlState = sockets.control?.readyState;
    const mediaState = sockets.media?.readyState;
    if (controlState === PHASE42_SOCKET_OPEN && mediaState === PHASE42_SOCKET_OPEN) {
      return PHASE42_SOCKET_OPEN;
    }
    if (controlState === SOCKET_CLOSED || mediaState === SOCKET_CLOSED) {
      return SOCKET_CLOSED;
    }
    if (controlState === SOCKET_CLOSING || mediaState === SOCKET_CLOSING) {
      return SOCKET_CLOSING;
    }
    return SOCKET_CONNECTING;
  }

  function maybeDispatchOpen(event: Phase42SocketEvent): void {
    if (
      !openEmitted &&
      !closeRequested &&
      sockets.control?.readyState === PHASE42_SOCKET_OPEN &&
      sockets.media?.readyState === PHASE42_SOCKET_OPEN
    ) {
      openEmitted = true;
      dispatch('open', event);
      while (preOpenMessages.length > 0 && !closeRequested) {
        const message = preOpenMessages.shift();
        if (message) dispatch('message', message);
      }
    }
  }

  function dispatchMessage(event: Phase42SocketEvent): void {
    if (!openEmitted) {
      preOpenMessages.push(event);
      return;
    }
    dispatch('message', event);
  }

  function dispatchCloseOnce(event: Phase42SocketEvent): void {
    if (closeEmitted) return;
    closeRequested = true;
    closeEmitted = true;
    closeSocket(sockets.control);
    closeSocket(sockets.media);
    dispatch('close', event);
  }

  function socketFactory(url: string, role: Phase42SocketRole): Phase42SocketLike {
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

  const splitOptions: Phase42SplitSocketClientOptions = {
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
  const client = createPhase42SplitSocketClient(splitOptions);

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
    set onopen(handler: Phase42AdapterEventHandler | null) {
      onopen = handler;
    },
    get onmessage() {
      return onmessage;
    },
    set onmessage(handler: Phase42AdapterEventHandler | null) {
      onmessage = handler;
    },
    get onclose() {
      return onclose;
    },
    set onclose(handler: Phase42AdapterEventHandler | null) {
      onclose = handler;
    },
    get onerror() {
      return onerror;
    },
    set onerror(handler: Phase42AdapterEventHandler | null) {
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
          throw new Error('phase42 control socket is not open');
        }
        return;
      }
      if (data instanceof ArrayBuffer) {
        if (sockets.media?.readyState !== PHASE42_SOCKET_OPEN) {
          throw new Error('phase42 media socket is not open');
        }
        sockets.media.send(data);
        return;
      }
      throw new Error('phase42 legacy socket adapter only supports string and ArrayBuffer payloads');
    },
  };
}
