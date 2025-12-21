import type { IncomingMessage } from 'http';
import crypto from 'crypto';
import WebSocket from 'ws';
import type { IWebSocketConnection, WebSocketStateValue } from './IWebSocketConnection';
import { WebSocketState } from './IWebSocketConnection';

/**
 * Wrapper around ws.WebSocket that implements IWebSocketConnection
 */
export class WsConnection implements IWebSocketConnection {
    public readonly id: string;
    private messageHandlers: Array<(data: Uint8Array) => void> = [];
    private closeHandlers: Array<(code: number, reason: string) => void> = [];
    private errorHandlers: Array<(error: Error) => void> = [];

    constructor(
        private readonly ws: WebSocket,
        private readonly request: IncomingMessage | undefined,
        id?: string,
    ) {
        this.id = id ?? crypto.randomUUID();
        this.setupEventHandlers();
    }

    private setupEventHandlers(): void {
        this.ws.on('message', (data: WebSocket.RawData) => {
            const buffer = this.normalizeData(data);
            for (const handler of this.messageHandlers) {
                handler(buffer);
            }
        });

        this.ws.on('close', (code: number, reason: Buffer) => {
            const reasonStr = reason.toString('utf-8');
            for (const handler of this.closeHandlers) {
                handler(code, reasonStr);
            }
        });

        this.ws.on('error', (error: Error) => {
            for (const handler of this.errorHandlers) {
                handler(error);
            }
        });
    }

    private normalizeData(data: WebSocket.RawData): Uint8Array {
        if (Buffer.isBuffer(data)) {
            return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        }
        if (data instanceof ArrayBuffer) {
            return new Uint8Array(data);
        }
        if (Array.isArray(data)) {
            // Array of Buffers - concatenate
            const totalLength = data.reduce((sum, buf) => sum + buf.length, 0);
            const result = new Uint8Array(totalLength);
            let offset = 0;
            for (const buf of data) {
                result.set(new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength), offset);
                offset += buf.length;
            }
            return result;
        }
        // Fallback - should not happen
        return new Uint8Array(0);
    }

    get readyState(): WebSocketStateValue {
        switch (this.ws.readyState) {
            case WebSocket.CONNECTING:
                return WebSocketState.CONNECTING;
            case WebSocket.OPEN:
                return WebSocketState.OPEN;
            case WebSocket.CLOSING:
                return WebSocketState.CLOSING;
            case WebSocket.CLOSED:
                return WebSocketState.CLOSED;
            default:
                return WebSocketState.CLOSED;
        }
    }

    send(data: Uint8Array | ArrayBuffer | string): boolean {
        if (this.ws.readyState !== WebSocket.OPEN) {
            return false;
        }
        try {
            this.ws.send(data);
            return true;
        } catch {
            return false;
        }
    }

    close(code = 1000, reason = ''): void {
        if (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING) {
            this.ws.close(code, reason);
        }
    }

    terminate(): void {
        this.ws.terminate();
    }

    getBufferedAmount(): number {
        return this.ws.bufferedAmount ?? 0;
    }

    cork(callback: () => void): void {
        // ws does not support cork - just execute callback
        callback();
    }

    getRemoteAddress(): string {
        // Try to get from request headers first (for proxied connections)
        const forwarded = this.request?.headers['x-forwarded-for'];
        if (forwarded) {
            const first = Array.isArray(forwarded) ? forwarded[0] : forwarded.split(',')[0];
            return first?.trim() ?? '';
        }
        // Fall back to socket remote address
        return this.request?.socket?.remoteAddress ?? '';
    }

    getRequest(): IncomingMessage | undefined {
        return this.request;
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
     * Get the underlying ws WebSocket instance
     * Used for compatibility with existing code during migration
     */
    getRawSocket(): WebSocket {
        return this.ws;
    }
}
