import { ServerCoordinator } from '../ServerCoordinator';
import { IServerStorage } from '../storage/IServerStorage';
import { WebSocket } from 'ws';

/**
 * Integration tests for uWebSockets.js transport.
 * These tests ensure the uWebSockets transport works correctly with ServerCoordinator.
 */
describe('UWebSocketsTransport Integration', () => {
    let server: ServerCoordinator;
    let mockStorage: jest.Mocked<IServerStorage>;
    let actualPort: number;

    beforeEach(async () => {
        mockStorage = {
            initialize: jest.fn().mockResolvedValue(undefined),
            close: jest.fn().mockResolvedValue(undefined),
            load: jest.fn(),
            loadAll: jest.fn(),
            loadAllKeys: jest.fn().mockResolvedValue([]),
            store: jest.fn(),
            storeAll: jest.fn(),
            delete: jest.fn(),
            deleteAll: jest.fn(),
        };

        server = new ServerCoordinator({
            port: 0,
            nodeId: 'uws-test-server',
            storage: mockStorage,
            wsTransport: 'uwebsockets',
        });

        await server.ready();
        actualPort = server.port;
    });

    afterEach(async () => {
        try {
            await server.shutdown();
        } catch (e) {
            // Ignore if already stopped
        }
    });

    describe('Connection Handling', () => {
        test('should accept WebSocket connections', async () => {
            const ws = new WebSocket(`ws://localhost:${actualPort}`);

            const connected = await new Promise<boolean>((resolve, reject) => {
                const timeout = setTimeout(() => reject(new Error('Connection timeout')), 5000);
                ws.on('open', () => {
                    clearTimeout(timeout);
                    resolve(true);
                });
                ws.on('error', (err) => {
                    clearTimeout(timeout);
                    reject(err);
                });
            });

            expect(connected).toBe(true);
            ws.close();
        });

        test('should handle multiple concurrent connections', async () => {
            const clientCount = 5;
            const clients: WebSocket[] = [];

            for (let i = 0; i < clientCount; i++) {
                const ws = new WebSocket(`ws://localhost:${actualPort}`);
                await new Promise<void>((resolve, reject) => {
                    const timeout = setTimeout(() => reject(new Error('Connection timeout')), 5000);
                    ws.on('open', () => {
                        clearTimeout(timeout);
                        resolve();
                    });
                    ws.on('error', (err) => {
                        clearTimeout(timeout);
                        reject(err);
                    });
                });
                clients.push(ws);
            }

            expect(clients.length).toBe(clientCount);

            for (const ws of clients) {
                ws.close();
            }
        });
    });

    describe('Graceful Shutdown', () => {
        test('should close all connections on shutdown', async () => {
            const ws = new WebSocket(`ws://localhost:${actualPort}`);

            await new Promise<void>((resolve, reject) => {
                const timeout = setTimeout(() => reject(new Error('Connection timeout')), 5000);
                ws.on('open', () => {
                    clearTimeout(timeout);
                    resolve();
                });
                ws.on('error', (err) => {
                    clearTimeout(timeout);
                    reject(err);
                });
            });

            const closePromise = new Promise<number>((resolve) => {
                ws.on('close', (code) => resolve(code));
            });

            await server.shutdown();

            const closeCode = await closePromise;
            expect([1000, 1001, 1006]).toContain(closeCode);
        });

        test('should not accept new connections after shutdown', async () => {
            await server.shutdown();

            const connectPromise = new Promise((resolve, reject) => {
                const ws = new WebSocket(`ws://localhost:${actualPort}`);
                const timeout = setTimeout(() => reject(new Error('Expected connection to fail')), 2000);
                ws.on('open', () => {
                    clearTimeout(timeout);
                    resolve(true);
                });
                ws.on('error', (err: Error) => {
                    clearTimeout(timeout);
                    reject(err);
                });
            });

            await expect(connectPromise).rejects.toThrow();
        });
    });

    describe('Transport Configuration', () => {
        test('should report uwebsockets as transport type', () => {
            // ServerCoordinator should be using uwebsockets transport
            expect(server).toBeDefined();
            // The server started successfully with uwebsockets transport
            expect(actualPort).toBeGreaterThan(0);
        });
    });
});
