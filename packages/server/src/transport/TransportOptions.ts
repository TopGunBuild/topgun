import type { TLSConfig } from '../types/TLSConfig';

/**
 * Configuration options for WebSocket transports
 */
export interface TransportOptions {
    /**
     * Port to listen on
     */
    port: number;

    /**
     * Host to bind to
     * @default '0.0.0.0'
     */
    host?: string;

    /**
     * TLS configuration for secure connections (wss://)
     */
    tls?: TLSConfig;

    /**
     * Connection backlog for pending connections
     * @default 511
     */
    backlog?: number;

    /**
     * Maximum message payload size in bytes
     * @default 67108864 (64MB)
     */
    maxPayload?: number;

    /**
     * Enable per-message deflate compression
     * Increases CPU usage but reduces bandwidth
     * @default false
     */
    compression?: boolean;

    /**
     * Idle timeout in seconds before closing connection
     * @default 120
     */
    idleTimeout?: number;

    /**
     * Maximum backpressure before dropping messages (uWS specific)
     * @default 16777216 (16MB)
     */
    maxBackpressure?: number;

    /**
     * Skip UTF-8 validation for text messages (performance optimization)
     * @default true
     */
    skipUTF8Validation?: boolean;
}

/**
 * HTTP request information passed to connection handler
 */
export interface IncomingRequest {
    /**
     * Request URL path
     */
    url: string;

    /**
     * Request headers
     */
    headers: Record<string, string | string[] | undefined>;

    /**
     * Query string parameters
     */
    query: string;

    /**
     * Remote IP address
     */
    remoteAddress: string;
}

/**
 * Handler for new WebSocket connections
 */
export type ConnectionHandler = (
    connection: import('./IWebSocketConnection').IWebSocketConnection,
    request: IncomingRequest,
) => void;

/**
 * Handler for transport errors
 */
export type ErrorHandler = (error: Error) => void;

/**
 * Handler for HTTP requests (for metrics, health endpoints)
 */
export interface HttpHandler {
    method: 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH' | 'OPTIONS';
    path: string;
    handler: (
        request: IncomingRequest,
        respond: (status: number, headers: Record<string, string>, body: string) => void,
    ) => void | Promise<void>;
}
