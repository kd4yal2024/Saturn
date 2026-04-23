import type { RemoteController } from '../controller/types';

export type RealtimeSocketLike = {
  addEventListener: (type: 'message' | 'open' | 'close' | 'error', listener: (event: any) => void) => void;
  close?: () => void;
};

export type SessionMessageResult =
  | { kind: 'text'; accepted: true; ready: boolean; receivedIqLogMessage: string | null }
  | { kind: 'binary'; accepted: boolean; frameKind: 'iq' | 'audio' | null; receivedIqLogMessage: string | null }
  | { kind: 'ignored'; accepted: false; receivedIqLogMessage: null };

export type RemoteSession = {
  bind: (socket: RealtimeSocketLike) => void;
  handleMessage: (data: unknown, nowMs?: number) => SessionMessageResult;
  getController: () => RemoteController;
};

export function createRemoteSession(controller: RemoteController): RemoteSession {
  return {
    bind(socket) {
      socket.addEventListener('message', (event) => {
        this.handleMessage(event.data, Date.now());
      });
    },
    handleMessage(data, nowMs = Date.now()) {
      if (typeof data === 'string') {
        const result = controller.applyTciText(data);
        return {
          kind: 'text',
          accepted: true,
          ready: result.events.ready,
          receivedIqLogMessage: result.events.receivedIqLogMessage,
        };
      }

      if (data instanceof ArrayBuffer) {
        const result = controller.applyBinaryFrame(data, nowMs);
        return {
          kind: 'binary',
          accepted: result.accepted,
          frameKind: result.kind,
          receivedIqLogMessage: result.receivedIqLogMessage,
        };
      }

      return {
        kind: 'ignored',
        accepted: false,
        receivedIqLogMessage: null,
      };
    },
    getController() {
      return controller;
    },
  };
}
