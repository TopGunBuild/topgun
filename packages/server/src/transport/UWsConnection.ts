import type * as uWS from 'uWebSockets.js';
import type { IWebSocketConnection, WebSocketStateValue } from './IWebSocketConnection';
import { WebSocketState } from './IWebSocketConnection';

/**
 * User data stored in uWebSockets.js WebSocket
 */
export interface UWsUserData {
    id: string;
    remoteAddress: string;
    url: string;
    headers: Record<string, string>;
    query: string;
    closed: boolean;
}

/**
 * Wrapper around uWebSockets.js WebSocket that implements IWebSocketConnection
 */
export class UWsConnection implements IWebSocketConnection {
    private messageHandlers: Array<(data: Uint8Array) => void> = [];
    private closeHandlers: Array<(code: number, reason: string) => void> = [];
    private errorHandlers: Array<(error: Error) => void> = [];
    private _closed = false;

    constructor(private readonly ws: uWS.WebSocket<UWsUserData>) {}

    get id(): string {
        return this.ws.getUserData().id;
    }

    get readyState(): WebSocketStateValue {
        // uWebSockets.js doesn't expose readyState, we track it ourselves
        if (this._closed || this.ws.getUserData().closed) {
            return WebSocketState.CLOSED;
        }
        return WebSocketState.OPEN;
    }

    send(data: Uint8Array | ArrayBuffer | string): boolean {
        if (this._closed) {
            return false;
        }
        try {
            // uWS.send returns:
            // 0 = dropped (backpressure)
            // 1 = success (buffered or sent)
            // 2 = dropped (socket closed)
            const result = this.ws.send(data, true); // true = isBinary
            return result === 1;
        } catch {
            return false;
        }
    }

    close(code = 1000, reason = ''): void {
        if (this._closed) {
            return;
        }
        try {
            this.ws.end(code, reason);
        } catch {
            // Socket may already be closed
        }
        this._closed = true;
    }

    terminate(): void {
        if (this._closed) {
            return;
        }
        try {
            this.ws.close();
        } catch {
            // Socket may already be closed
        }
        this._closed = true;
    }

    getBufferedAmount(): number {
        if (this._closed) {
            return 0;
        }
        try {
            return this.ws.getBufferedAmount();
        } catch {
            return 0;
        }
    }

    cork(callback: () => void): void {
        if (this._closed) {
            callback();
            return;
        }
        try {
            this.ws.cork(callback);
        } catch {
            callback();
        }
    }

    getRemoteAddress(): string {
        return this.ws.getUserData().remoteAddress;
    }

    getRequest(): undefined {
        // uWebSockets.js doesn't provide access to the original request after upgrade
        return undefined;
    }

    onMessage(handler: (data: Uint8Array) => void): void {
        this.messageHandlers.push(handler);
    }

    onClose(handler: (code: number, reason: string) => void): void {
        this.closeHandlers.push(handler);
    }

    onError(handler: (error: Error) => void): void {
        this.errorHandlers.push(handler);
    }

    /**
     * Called by UWebSocketsTransport when a message is received
     * @internal
     */
    _handleMessage(data: ArrayBuffer): void {
        const uint8 = new Uint8Array(data);
        for (const handler of this.messageHandlers) {
            try {
                handler(uint8);
            } catch (error) {
                this._handleError(error as Error);
            }
        }
    }

    /**
     * Called by UWebSocketsTransport when the connection is closed
     * @internal
     */
    _handleClose(code: number, reason: ArrayBuffer): void {
        this._closed = true;
        const reasonStr = Buffer.from(reason).toString('utf-8');
        for (const handler of this.closeHandlers) {
            try {
                handler(code, reasonStr);
            } catch {
                // Ignore errors in close handlers
            }
        }
    }

    /**
     * Called by UWebSocketsTransport when an error occurs
     * @internal
     */
    _handleError(error: Error): void {
        for (const handler of this.errorHandlers) {
            try {
                handler(error);
            } catch {
                // Ignore errors in error handlers
            }
        }
    }

    /**
     * Subscribe to a topic (uWS pub/sub)
     */
    subscribe(topic: string): boolean {
        if (this._closed) {
            return false;
        }
        try {
            return this.ws.subscribe(topic);
        } catch {
            return false;
        }
    }

    /**
     * Unsubscribe from a topic
     */
    unsubscribe(topic: string): boolean {
        if (this._closed) {
            return false;
        }
        try {
            return this.ws.unsubscribe(topic);
        } catch {
            return false;
        }
    }

    /**
     * Check if subscribed to a topic
     */
    isSubscribed(topic: string): boolean {
        if (this._closed) {
            return false;
        }
        try {
            return this.ws.isSubscribed(topic);
        } catch {
            return false;
        }
    }

    /**
     * Get the underlying uWebSockets.js WebSocket
     * Used for advanced operations
     */
    getRawSocket(): uWS.WebSocket<UWsUserData> {
        return this.ws;
    }
}
