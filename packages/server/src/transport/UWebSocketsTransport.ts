import * as uWS from 'uWebSockets.js';
import * as crypto from 'crypto';
import { readFileSync } from 'fs';
import type { IWebSocketTransport } from './IWebSocketTransport';
import type { ConnectionHandler, ErrorHandler, HttpHandler, IncomingRequest, TransportOptions } from './TransportOptions';
import { UWsConnection, type UWsUserData } from './UWsConnection';

/**
 * WebSocket transport implementation using uWebSockets.js
 *
 * Features:
 * - High performance C++ WebSocket server
 * - Built-in backpressure handling via getBufferedAmount()
 * - cork() for batching multiple sends
 * - Integrated HTTP server for metrics/health endpoints
 */
export class UWebSocketsTransport implements IWebSocketTransport {
    private app: uWS.TemplatedApp | null = null;
    private listenSocket: uWS.us_listen_socket | null = null;
    private options: TransportOptions | null = null;
    private running = false;
    private actualPort = 0;

    private connectionHandler: ConnectionHandler | null = null;
    private errorHandler: ErrorHandler | null = null;
    private httpHandlers: HttpHandler[] = [];
    private connections = new Map<string, UWsConnection>();

    async start(options: TransportOptions): Promise<void> {
        if (this.running) {
            throw new Error('Transport is already running');
        }

        this.options = options;

        // Create App or SSLApp based on TLS config
        if (options.tls?.enabled) {
            this.app = uWS.SSLApp({
                key_file_name: options.tls.keyPath,
                cert_file_name: options.tls.certPath,
                ca_file_name: options.tls.caCertPath,
                passphrase: options.tls.passphrase,
                // ssl_prefer_low_memory_usage: true, // Optional: reduce memory for many connections
            });
        } else {
            this.app = uWS.App();
        }

        // Configure WebSocket behavior
        this.app.ws<UWsUserData>('/*', {
            // Compression
            compression: options.compression ? uWS.SHARED_COMPRESSOR : uWS.DISABLED,

            // Limits
            maxPayloadLength: options.maxPayload ?? 64 * 1024 * 1024, // 64MB
            maxBackpressure: options.maxBackpressure ?? 16 * 1024 * 1024, // 16MB
            idleTimeout: options.idleTimeout ?? 120, // 120 seconds

            // Upgrade handler - extract request info before upgrade
            upgrade: (res, req, context) => {
                const url = req.getUrl();
                const query = req.getQuery();

                // Extract headers
                const headers: Record<string, string> = {};
                req.forEach((key, value) => {
                    headers[key] = value;
                });

                // Get remote address
                let remoteAddress = '';
                const forwarded = headers['x-forwarded-for'];
                if (forwarded) {
                    remoteAddress = forwarded.split(',')[0]?.trim() ?? '';
                }
                if (!remoteAddress) {
                    // Get IP from socket
                    const addressBuffer = res.getRemoteAddressAsText();
                    remoteAddress = Buffer.from(addressBuffer).toString();
                }

                // Generate connection ID
                const id = crypto.randomUUID();

                // Store user data for later access
                const userData: UWsUserData = {
                    id,
                    remoteAddress,
                    url,
                    headers,
                    query,
                    closed: false,
                };

                // Complete the upgrade
                res.upgrade(
                    userData,
                    req.getHeader('sec-websocket-key'),
                    req.getHeader('sec-websocket-protocol'),
                    req.getHeader('sec-websocket-extensions'),
                    context,
                );
            },

            // Connection opened
            open: (ws) => {
                const userData = ws.getUserData();
                const connection = new UWsConnection(ws);
                this.connections.set(userData.id, connection);

                // Build request info
                const request: IncomingRequest = {
                    url: userData.url,
                    headers: userData.headers,
                    query: userData.query,
                    remoteAddress: userData.remoteAddress,
                };

                // Notify handler
                if (this.connectionHandler) {
                    try {
                        this.connectionHandler(connection, request);
                    } catch (error) {
                        if (this.errorHandler) {
                            this.errorHandler(error as Error);
                        }
                    }
                }
            },

            // Message received
            message: (ws, message, isBinary) => {
                const userData = ws.getUserData();
                const connection = this.connections.get(userData.id);
                if (connection) {
                    connection._handleMessage(message);
                }
            },

            // Connection closed
            close: (ws, code, message) => {
                const userData = ws.getUserData();
                userData.closed = true;

                const connection = this.connections.get(userData.id);
                if (connection) {
                    connection._handleClose(code, message);
                    this.connections.delete(userData.id);
                }
            },

            // Backpressure drained - can resume sending
            drain: (ws) => {
                // Could emit an event here if needed for flow control
                // For now, connections can check getBufferedAmount() themselves
            },
        });

        // Register HTTP handlers
        for (const handler of this.httpHandlers) {
            this.registerHttpHandler(handler);
        }

        // Default health endpoint
        this.app.get('/health', (res, req) => {
            res.writeStatus('200 OK');
            res.writeHeader('Content-Type', 'text/plain');
            res.end('OK');
        });

        // Start listening
        return new Promise((resolve, reject) => {
            const port = options.port;
            const host = options.host ?? '0.0.0.0';

            this.app!.listen(host, port, (socket) => {
                if (socket) {
                    this.listenSocket = socket;
                    this.actualPort = uWS.us_socket_local_port(socket);
                    this.running = true;
                    resolve();
                } else {
                    reject(new Error(`Failed to listen on ${host}:${port}`));
                }
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
            for (const conn of this.connections.values()) {
                conn.close(1001, 'Server Shutdown');
            }
            // Give connections time to close
            await new Promise((resolve) => setTimeout(resolve, 100));
        } else {
            for (const conn of this.connections.values()) {
                conn.terminate();
            }
        }

        this.connections.clear();

        // Close listen socket
        if (this.listenSocket) {
            uWS.us_listen_socket_close(this.listenSocket);
            this.listenSocket = null;
        }

        this.app = null;
    }

    onConnection(handler: ConnectionHandler): void {
        this.connectionHandler = handler;
    }

    onError(handler: ErrorHandler): void {
        this.errorHandler = handler;
    }

    addHttpHandler(handler: HttpHandler): void {
        this.httpHandlers.push(handler);

        // If already running, register immediately
        if (this.app && this.running) {
            this.registerHttpHandler(handler);
        }
    }

    private registerHttpHandler(handler: HttpHandler): void {
        if (!this.app) return;

        const uWsHandler = (res: uWS.HttpResponse, req: uWS.HttpRequest) => {
            // Handle aborted requests
            let aborted = false;
            res.onAborted(() => {
                aborted = true;
            });

            // Build request info
            const url = req.getUrl();
            const query = req.getQuery();
            const headers: Record<string, string> = {};
            req.forEach((key, value) => {
                headers[key] = value;
            });

            // Get remote address
            let remoteAddress = '';
            const forwarded = headers['x-forwarded-for'];
            if (forwarded) {
                remoteAddress = forwarded.split(',')[0]?.trim() ?? '';
            }
            if (!remoteAddress) {
                const addressBuffer = res.getRemoteAddressAsText();
                remoteAddress = Buffer.from(addressBuffer).toString();
            }

            const request: IncomingRequest = {
                url,
                headers,
                query,
                remoteAddress,
            };

            const respond = (status: number, respHeaders: Record<string, string>, body: string) => {
                if (aborted) return;

                res.cork(() => {
                    res.writeStatus(status.toString());
                    for (const [key, value] of Object.entries(respHeaders)) {
                        res.writeHeader(key, value);
                    }
                    res.end(body);
                });
            };

            try {
                const result = handler.handler(request, respond);
                if (result instanceof Promise) {
                    result.catch((error) => {
                        if (!aborted) {
                            res.writeStatus('500');
                            res.end('Internal Server Error');
                        }
                        if (this.errorHandler) {
                            this.errorHandler(error);
                        }
                    });
                }
            } catch (error) {
                if (!aborted) {
                    res.writeStatus('500');
                    res.end('Internal Server Error');
                }
                if (this.errorHandler) {
                    this.errorHandler(error as Error);
                }
            }
        };

        // Register based on method
        switch (handler.method) {
            case 'GET':
                this.app.get(handler.path, uWsHandler);
                break;
            case 'POST':
                this.app.post(handler.path, uWsHandler);
                break;
            case 'PUT':
                this.app.put(handler.path, uWsHandler);
                break;
            case 'DELETE':
                this.app.del(handler.path, uWsHandler);
                break;
            case 'PATCH':
                this.app.patch(handler.path, uWsHandler);
                break;
            case 'OPTIONS':
                this.app.options(handler.path, uWsHandler);
                break;
        }
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

    /**
     * Publish to a topic (uWS pub/sub)
     * All connections subscribed to the topic will receive the message
     */
    publish(topic: string, data: Uint8Array | string): boolean {
        if (!this.app || !this.running) {
            return false;
        }
        try {
            return this.app.publish(topic, data, true); // true = isBinary
        } catch {
            return false;
        }
    }

    /**
     * Get connection by ID
     */
    getConnection(id: string): UWsConnection | undefined {
        return this.connections.get(id);
    }

    /**
     * Get all connections
     */
    getAllConnections(): Map<string, UWsConnection> {
        return this.connections;
    }
}
