import type { IncomingMessage } from 'http';

/**
 * WebSocket connection states (compatible with standard WebSocket)
 */
export const WebSocketState = {
    CONNECTING: 0,
    OPEN: 1,
    CLOSING: 2,
    CLOSED: 3,
} as const;

export type WebSocketStateValue = (typeof WebSocketState)[keyof typeof WebSocketState];

/**
 * Abstract interface for a WebSocket connection.
 * Allows swapping between ws and uWebSockets.js implementations.
 */
export interface IWebSocketConnection {
    /**
     * Unique identifier for this connection
     */
    readonly id: string;

    /**
     * Current connection state (CONNECTING, OPEN, CLOSING, CLOSED)
     */
    readonly readyState: WebSocketStateValue;

    /**
     * Send data to the client
     * @param data - Binary or string data to send
     * @returns true if sent successfully, false if failed or dropped
     */
    send(data: Uint8Array | ArrayBuffer | string): boolean;

    /**
     * Close the connection gracefully
     * @param code - WebSocket close code (default: 1000)
     * @param reason - Close reason string
     */
    close(code?: number, reason?: string): void;

    /**
     * Forcefully terminate the connection without handshake
     */
    terminate(): void;

    /**
     * Get the amount of bytes buffered for sending
     * Used for backpressure handling
     */
    getBufferedAmount(): number;

    /**
     * Cork multiple sends into a single syscall (uWS optimization)
     * For ws, this just executes the callback immediately
     */
    cork(callback: () => void): void;

    /**
     * Get remote IP address
     */
    getRemoteAddress(): string;

    /**
     * Get original HTTP request (for headers, cookies, etc.)
     * May be undefined for uWebSockets.js
     */
    getRequest(): IncomingMessage | undefined;

    /**
     * Subscribe to message events
     */
    onMessage(handler: (data: Uint8Array) => void): void;

    /**
     * Subscribe to close events
     */
    onClose(handler: (code: number, reason: string) => void): void;

    /**
     * Subscribe to error events
     */
    onError(handler: (error: Error) => void): void;
}
