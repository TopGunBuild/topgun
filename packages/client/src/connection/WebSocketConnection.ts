import type { IConnection } from '../types';

/**
 * Ready-state constants matching the WebSocket spec values.
 * Allows callers to compare readyState without depending on the WebSocket global.
 */
export const ConnectionReadyState = {
  CONNECTING: 0,
  OPEN: 1,
  CLOSING: 2,
  CLOSED: 3,
} as const;

/**
 * Thin adapter that wraps a raw WebSocket and exposes only the IConnection
 * surface. This keeps the concrete WebSocket type out of public return types
 * while adding zero overhead.
 */
export class WebSocketConnection implements IConnection {
  constructor(private readonly ws: WebSocket) {}

  send(data: ArrayBuffer | Uint8Array | string): void {
    this.ws.send(data);
  }

  close(): void {
    this.ws.close();
  }

  get readyState(): number {
    return this.ws.readyState;
  }
}
