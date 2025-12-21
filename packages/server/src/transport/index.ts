/**
 * WebSocket Transport Abstraction Layer
 *
 * Provides a unified interface for different WebSocket implementations:
 * - ws (Node.js, stable, widely used)
 * - uWebSockets.js (high performance, lower latency)
 *
 * Usage:
 * ```typescript
 * const transport = createTransport('ws'); // or 'uwebsockets'
 * await transport.start({ port: 8080 });
 *
 * transport.onConnection((conn, req) => {
 *   conn.onMessage((data) => {
 *     // Handle message
 *   });
 * });
 * ```
 */

export { type IWebSocketConnection, WebSocketState, type WebSocketStateValue } from './IWebSocketConnection';
export { type IWebSocketTransport, type TransportType } from './IWebSocketTransport';
export {
    type TransportOptions,
    type IncomingRequest,
    type ConnectionHandler,
    type ErrorHandler,
    type HttpHandler,
} from './TransportOptions';

export { WsConnection } from './WsConnection';
export { WsTransport } from './WsTransport';
export { UWsConnection, type UWsUserData } from './UWsConnection';
export { UWebSocketsTransport } from './UWebSocketsTransport';

import type { IWebSocketTransport, TransportType } from './IWebSocketTransport';
import { WsTransport } from './WsTransport';
import { UWebSocketsTransport } from './UWebSocketsTransport';

/**
 * Create a WebSocket transport instance
 *
 * @param type - Transport type: 'ws' (default) or 'uwebsockets'
 * @returns Transport instance
 */
export function createTransport(type: TransportType = 'ws'): IWebSocketTransport {
    switch (type) {
        case 'ws':
            return new WsTransport();
        case 'uwebsockets':
            return new UWebSocketsTransport();
        default:
            throw new Error(`Unknown transport type: ${type}`);
    }
}

/**
 * Check if a transport type is available
 */
export function isTransportAvailable(type: TransportType): boolean {
    switch (type) {
        case 'ws':
            return true;
        case 'uwebsockets':
            return true;
        default:
            return false;
    }
}
