import { createServer as createHttpServer, type Server as HttpServer, type IncomingMessage, type ServerResponse } from 'http';
import { createServer as createHttpsServer, type Server as HttpsServer } from 'https';
import { readFileSync } from 'fs';
import { WebSocketServer, type WebSocket } from 'ws';
import { URL } from 'url';
import type { IWebSocketTransport } from './IWebSocketTransport';
import type { ConnectionHandler, ErrorHandler, HttpHandler, IncomingRequest, TransportOptions } from './TransportOptions';
import { WsConnection } from './WsConnection';

/**
 * WebSocket transport implementation using ws library
 */
export class WsTransport implements IWebSocketTransport {
    private wss: WebSocketServer | null = null;
    private httpServer: HttpServer | HttpsServer | null = null;
    private options: TransportOptions | null = null;
    private running = false;
    private actualPort = 0;

    private connectionHandler: ConnectionHandler | null = null;
    private errorHandler: ErrorHandler | null = null;
    private httpHandlers: HttpHandler[] = [];
    private connections = new Map<string, WsConnection>();

    async start(options: TransportOptions): Promise<void> {
        if (this.running) {
            throw new Error('Transport is already running');
        }

        this.options = options;

        // Create HTTP(S) server
        if (options.tls?.enabled) {
            const tlsOptions = this.buildTLSOptions(options);
            this.httpServer = createHttpsServer(tlsOptions, (req, res) => this.handleHttpRequest(req, res));
        } else {
            this.httpServer = createHttpServer((req, res) => this.handleHttpRequest(req, res));
        }

        // Configure HTTP server
        this.httpServer.maxConnections = 10000;
        this.httpServer.timeout = 120000;
        this.httpServer.keepAliveTimeout = 5000;
        this.httpServer.headersTimeout = 60000;

        // Optimize socket settings
        this.httpServer.on('connection', (socket) => {
            socket.setNoDelay(true);
            socket.setKeepAlive(true, 60000);
        });

        // Create WebSocket server
        this.wss = new WebSocketServer({
            server: this.httpServer,
            backlog: options.backlog ?? 511,
            perMessageDeflate: options.compression ?? false,
            maxPayload: options.maxPayload ?? 64 * 1024 * 1024,
            skipUTF8Validation: options.skipUTF8Validation ?? true,
        });

        // Handle WebSocket connections
        this.wss.on('connection', (ws: WebSocket, req: IncomingMessage) => {
            this.handleConnection(ws, req);
        });

        this.wss.on('error', (error: Error) => {
            if (this.errorHandler) {
                this.errorHandler(error);
            }
        });

        // Start listening
        return new Promise((resolve, reject) => {
            const port = options.port;
            const host = options.host ?? '0.0.0.0';

            this.httpServer!.listen(port, host, () => {
                const address = this.httpServer!.address();
                if (address && typeof address === 'object') {
                    this.actualPort = address.port;
                } else {
                    this.actualPort = port;
                }
                this.running = true;
                resolve();
            });

            this.httpServer!.on('error', (error: Error) => {
                reject(error);
            });
        });
    }

    async stop(graceful = true): Promise<void> {
        if (!this.running) {
            return;
        }

        this.running = false;

        // Close all connections
        if (graceful) {
            const closePromises: Promise<void>[] = [];
            for (const conn of this.connections.values()) {
                closePromises.push(
                    new Promise((resolve) => {
                        conn.onClose(() => resolve());
                        conn.close(1001, 'Server Shutdown');
                        // Fallback timeout
                        setTimeout(resolve, 5000);
                    }),
                );
            }
            await Promise.allSettled(closePromises);
        } else {
            for (const conn of this.connections.values()) {
                conn.terminate();
            }
        }

        this.connections.clear();

        // Close WebSocket server
        if (this.wss) {
            await new Promise<void>((resolve) => {
                this.wss!.close(() => resolve());
            });
            this.wss = null;
        }

        // Close HTTP server
        if (this.httpServer) {
            await new Promise<void>((resolve) => {
                this.httpServer!.close(() => resolve());
            });
            this.httpServer = null;
        }
    }

    onConnection(handler: ConnectionHandler): void {
        this.connectionHandler = handler;
    }

    onError(handler: ErrorHandler): void {
        this.errorHandler = handler;
    }

    addHttpHandler(handler: HttpHandler): void {
        this.httpHandlers.push(handler);
    }

    getConnectionCount(): number {
        return this.connections.size;
    }

    getTotalBufferedBytes(): number {
        let total = 0;
        for (const conn of this.connections.values()) {
            total += conn.getBufferedAmount();
        }
        return total;
    }

    isRunning(): boolean {
        return this.running;
    }

    getPort(): number {
        return this.actualPort;
    }

    broadcast(data: Uint8Array | string, filter?: (connId: string) => boolean): void {
        for (const [id, conn] of this.connections) {
            if (!filter || filter(id)) {
                conn.send(data);
            }
        }
    }

    private handleConnection(ws: WebSocket, req: IncomingMessage): void {
        const connection = new WsConnection(ws, req);
        this.connections.set(connection.id, connection);

        // Remove from map on close
        connection.onClose(() => {
            this.connections.delete(connection.id);
        });

        // Build request info
        const request = this.buildIncomingRequest(req);

        // Notify handler
        if (this.connectionHandler) {
            this.connectionHandler(connection, request);
        }
    }

    private handleHttpRequest(req: IncomingMessage, res: ServerResponse): void {
        const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);
        const method = req.method as HttpHandler['method'];

        // Find matching handler
        const handler = this.httpHandlers.find(
            (h) => h.method === method && h.path === url.pathname,
        );

        if (handler) {
            const request = this.buildIncomingRequest(req);
            const respond = (status: number, headers: Record<string, string>, body: string) => {
                res.writeHead(status, headers);
                res.end(body);
            };

            try {
                const result = handler.handler(request, respond);
                if (result instanceof Promise) {
                    result.catch((error) => {
                        res.writeHead(500);
                        res.end('Internal Server Error');
                        if (this.errorHandler) {
                            this.errorHandler(error);
                        }
                    });
                }
            } catch (error) {
                res.writeHead(500);
                res.end('Internal Server Error');
                if (this.errorHandler) {
                    this.errorHandler(error as Error);
                }
            }
        } else {
            // No handler - return 404 for non-WebSocket requests
            // WebSocket upgrade is handled by ws automatically
            if (req.headers.upgrade?.toLowerCase() !== 'websocket') {
                res.writeHead(404);
                res.end('Not Found');
            }
        }
    }

    private buildIncomingRequest(req: IncomingMessage): IncomingRequest {
        const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);

        // Get remote address
        const forwarded = req.headers['x-forwarded-for'];
        let remoteAddress = '';
        if (forwarded) {
            const first = Array.isArray(forwarded) ? forwarded[0] : forwarded.split(',')[0];
            remoteAddress = first?.trim() ?? '';
        }
        if (!remoteAddress) {
            remoteAddress = req.socket?.remoteAddress ?? '';
        }

        return {
            url: url.pathname,
            headers: req.headers as Record<string, string | string[] | undefined>,
            query: url.search.slice(1), // Remove leading '?'
            remoteAddress,
        };
    }

    private buildTLSOptions(options: TransportOptions): Record<string, unknown> {
        const tls = options.tls!;
        const tlsOptions: Record<string, unknown> = {
            cert: readFileSync(tls.certPath),
            key: readFileSync(tls.keyPath),
            minVersion: tls.minVersion ?? 'TLSv1.2',
        };

        if (tls.caCertPath) {
            tlsOptions.ca = readFileSync(tls.caCertPath);
        }
        if (tls.ciphers) {
            tlsOptions.ciphers = tls.ciphers;
        }
        if (tls.passphrase) {
            tlsOptions.passphrase = tls.passphrase;
        }

        return tlsOptions;
    }

    /**
     * Get connection by ID
     */
    getConnection(id: string): WsConnection | undefined {
        return this.connections.get(id);
    }

    /**
     * Get all connections
     */
    getAllConnections(): Map<string, WsConnection> {
        return this.connections;
    }
}
