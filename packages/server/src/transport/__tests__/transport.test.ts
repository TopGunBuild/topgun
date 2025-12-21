import WebSocket from 'ws';
import { createTransport, WsTransport, UWebSocketsTransport } from '../index';
import type { IWebSocketTransport, IWebSocketConnection, IncomingRequest } from '../index';

describe('Transport Factory', () => {
    it('should create WsTransport by default', () => {
        const transport = createTransport();
        expect(transport).toBeInstanceOf(WsTransport);
    });

    it('should create WsTransport when specified', () => {
        const transport = createTransport('ws');
        expect(transport).toBeInstanceOf(WsTransport);
    });

    it('should create UWebSocketsTransport when specified', () => {
        const transport = createTransport('uwebsockets');
        expect(transport).toBeInstanceOf(UWebSocketsTransport);
    });

    it('should throw for unknown transport type', () => {
        expect(() => createTransport('unknown' as any)).toThrow('Unknown transport type');
    });
});

describe.each([
    ['WsTransport', () => new WsTransport()],
    ['UWebSocketsTransport', () => new UWebSocketsTransport()],
])('%s', (name, createTransportFn) => {
    let transport: IWebSocketTransport;
    let port: number;

    beforeEach(async () => {
        transport = createTransportFn();
        await transport.start({ port: 0 }); // Random port
        port = transport.getPort();
    });

    afterEach(async () => {
        await transport.stop();
    });

    it('should start and listen on a port', () => {
        expect(transport.isRunning()).toBe(true);
        expect(port).toBeGreaterThan(0);
    });

    it('should accept WebSocket connections', async () => {
        const connectionPromise = new Promise<IWebSocketConnection>((resolve) => {
            transport.onConnection((conn) => {
                resolve(conn);
            });
        });

        const ws = new WebSocket(`ws://localhost:${port}`);

        // Wait for ws to be open before checking connection
        await new Promise<void>((resolve) => ws.on('open', resolve));

        const conn = await connectionPromise;
        expect(conn).toBeDefined();
        expect(conn.id).toBeDefined();

        ws.close();
        // Wait for close to complete
        await new Promise<void>((resolve) => ws.on('close', () => resolve()));
    });

    it('should receive messages from clients', async () => {
        const messagePromise = new Promise<Uint8Array>((resolve) => {
            transport.onConnection((conn) => {
                conn.onMessage((data) => {
                    resolve(data);
                });
            });
        });

        const ws = new WebSocket(`ws://localhost:${port}`);
        await new Promise<void>((resolve) => {
            ws.on('open', () => {
                ws.send(Buffer.from('hello'));
                resolve();
            });
        });

        const data = await messagePromise;
        expect(Buffer.from(data).toString()).toBe('hello');

        ws.close();
    });

    it('should send messages to clients', async () => {
        const messagePromise = new Promise<Buffer>((resolve) => {
            const ws = new WebSocket(`ws://localhost:${port}`);
            ws.binaryType = 'nodebuffer';
            ws.on('message', (data) => {
                resolve(data as Buffer);
                ws.close();
            });
        });

        transport.onConnection((conn) => {
            conn.send(Buffer.from('world'));
        });

        const data = await messagePromise;
        expect(data.toString()).toBe('world');
    });

    it('should handle connection close', async () => {
        const closePromise = new Promise<{ code: number; reason: string }>((resolve) => {
            transport.onConnection((conn) => {
                conn.onClose((code, reason) => {
                    resolve({ code, reason });
                });
            });
        });

        const ws = new WebSocket(`ws://localhost:${port}`);
        await new Promise<void>((resolve) => {
            ws.on('open', () => {
                ws.close(1000, 'bye');
                resolve();
            });
        });

        const result = await closePromise;
        expect(result.code).toBe(1000);
    });

    it('should track connection count', async () => {
        expect(transport.getConnectionCount()).toBe(0);

        const ws1 = new WebSocket(`ws://localhost:${port}`);
        await new Promise<void>((resolve) => ws1.on('open', resolve));

        // Wait for server to register connection
        await new Promise((resolve) => setTimeout(resolve, 50));
        expect(transport.getConnectionCount()).toBe(1);

        const ws2 = new WebSocket(`ws://localhost:${port}`);
        await new Promise<void>((resolve) => ws2.on('open', resolve));

        await new Promise((resolve) => setTimeout(resolve, 50));
        expect(transport.getConnectionCount()).toBe(2);

        ws1.close();
        await new Promise((resolve) => setTimeout(resolve, 50));
        expect(transport.getConnectionCount()).toBe(1);

        ws2.close();
        await new Promise((resolve) => setTimeout(resolve, 50));
        expect(transport.getConnectionCount()).toBe(0);
    });

    it('should handle HTTP endpoints', async () => {
        transport.addHttpHandler({
            method: 'GET',
            path: '/test',
            handler: (req, respond) => {
                respond(200, { 'Content-Type': 'application/json' }, '{"status":"ok"}');
            },
        });

        const response = await fetch(`http://localhost:${port}/test`);
        expect(response.status).toBe(200);
        const data = await response.json();
        expect(data.status).toBe('ok');
    });

    it('should broadcast to all connections', async () => {
        const messages: string[] = [];
        const sockets: WebSocket[] = [];

        // Create connections first and wait for them to open
        for (let i = 0; i < 2; i++) {
            const ws = new WebSocket(`ws://localhost:${port}`);
            ws.binaryType = 'nodebuffer';
            sockets.push(ws);
            await new Promise<void>((resolve) => ws.on('open', resolve));
        }

        // Wait for server to register connections
        await new Promise((resolve) => setTimeout(resolve, 100));

        // Setup message handlers
        const messagePromise = new Promise<void>((resolve) => {
            let count = 0;
            for (const ws of sockets) {
                ws.on('message', (data: Buffer) => {
                    messages.push(data.toString());
                    count++;
                    if (count === 2) resolve();
                });
            }
        });

        // Broadcast
        transport.broadcast(Buffer.from('broadcast'));

        await messagePromise;
        expect(messages).toHaveLength(2);
        expect(messages.every((m) => m === 'broadcast')).toBe(true);

        // Cleanup
        for (const ws of sockets) {
            ws.close();
        }
    });

    it('should stop gracefully', async () => {
        const ws = new WebSocket(`ws://localhost:${port}`);
        await new Promise<void>((resolve) => ws.on('open', resolve));

        const closePromise = new Promise<number>((resolve) => {
            ws.on('close', (code) => resolve(code));
        });

        await transport.stop(true);

        const code = await closePromise;
        expect(code).toBe(1001); // Server Shutdown
        expect(transport.isRunning()).toBe(false);
    });
});
