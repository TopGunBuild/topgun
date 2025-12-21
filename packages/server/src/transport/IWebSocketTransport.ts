import type { ConnectionHandler, ErrorHandler, HttpHandler, TransportOptions } from './TransportOptions';

/**
 * Abstract interface for WebSocket server transport.
 * Allows swapping between ws and uWebSockets.js implementations.
 */
export interface IWebSocketTransport {
    /**
     * Start the WebSocket server
     * @param options - Transport configuration
     */
    start(options: TransportOptions): Promise<void>;

    /**
     * Stop the server and close all connections
     * @param graceful - If true, notify clients before closing
     */
    stop(graceful?: boolean): Promise<void>;

    /**
     * Register connection handler
     */
    onConnection(handler: ConnectionHandler): void;

    /**
     * Register error handler
     */
    onError(handler: ErrorHandler): void;

    /**
     * Register HTTP endpoint handler (for metrics, health, etc.)
     */
    addHttpHandler(handler: HttpHandler): void;

    /**
     * Get current number of active connections
     */
    getConnectionCount(): number;

    /**
     * Get total bytes buffered across all connections
     */
    getTotalBufferedBytes(): number;

    /**
     * Check if transport is running
     */
    isRunning(): boolean;

    /**
     * Get the actual port the server is listening on
     * Useful when port 0 is used for random port assignment
     */
    getPort(): number;

    /**
     * Broadcast data to all connected clients
     * @param data - Data to send
     * @param filter - Optional filter function to select recipients
     */
    broadcast(data: Uint8Array | string, filter?: (connId: string) => boolean): void;
}

/**
 * Transport type identifier
 */
export type TransportType = 'ws' | 'uwebsockets';
