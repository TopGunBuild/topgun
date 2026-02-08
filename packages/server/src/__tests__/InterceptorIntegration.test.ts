import { ServerCoordinator, ServerFactory } from '../';
import { WebSocket } from 'ws';
import { createServer } from 'http';
import { IInterceptor, ServerOp, OpContext, ConnectionContext } from '../interceptor/IInterceptor';
import { serialize, deserialize } from '@topgunbuild/core';
import * as crypto from 'crypto';
import { pollUntil } from './utils/test-helpers';

// Mock Storage
const mockStorage: any = {
    initialize: jest.fn().mockResolvedValue(undefined),
    loadAllKeys: jest.fn().mockResolvedValue([]),
    store: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    deleteAll: jest.fn().mockResolvedValue(undefined)
};

describe('ServerCoordinator Interceptor Integration', () => {
    let server: ServerCoordinator;
    let port: number;
    let clientWs: WebSocket;

    const mockInterceptor: IInterceptor = {
        name: 'MockInterceptor',
        onConnection: jest.fn().mockResolvedValue(undefined),
        onDisconnect: jest.fn().mockResolvedValue(undefined),
        onBeforeOp: jest.fn(async (op) => op),
        onAfterOp: jest.fn().mockResolvedValue(undefined)
    };

    beforeEach(async () => {
        jest.clearAllMocks();
        
        server = ServerFactory.create({
            port: 0, // Random port
            nodeId: 'test-node',
            storage: mockStorage,
            interceptors: [mockInterceptor]
        });

        await server.ready();
        port = server.port;
    });

    afterEach(async () => {
        if (clientWs) {
            clientWs.close();
        }
        await server.shutdown();
    });

    const connectClient = (): Promise<WebSocket> => {
        return new Promise((resolve) => {
            const ws = new WebSocket(`ws://localhost:${port}`);
            ws.on('open', () => resolve(ws));
        });
    };

    const authenticate = (ws: WebSocket) => {
        // Mock auth flow if needed, or just rely on basic connection for now
        // ServerCoordinator requires AUTH message after connection usually
        // We can use a simplified flow or mock the message handling if possible, 
        // but here we are testing the public interface via WS.
        
        // We need a valid token logic or bypass. 
        // The ServerCoordinator uses jwt.verify. We should probably mock jwt or provide a valid token if strict.
        // Or we can rely on `onConnection` which runs *before* auth for the connection object creation,
        // but the `onBeforeOp` runs only after auth when processing messages.
        
        // Actually `onConnection` runs immediately upon socket connection.
    };
    
    // Helper to wait for socket state
    const waitForSocketState = (ws: WebSocket, state: number) => {
        return new Promise<void>(resolve => {
            if (ws.readyState === state) resolve();
            else {
                ws.once(state === WebSocket.OPEN ? 'open' : 'close', () => resolve());
            }
        });
    }

    test('should call onConnection when client connects', async () => {
        clientWs = await connectClient();
        // Wait for server to process the connection and call the interceptor
        await pollUntil(
          () => (mockInterceptor.onConnection as jest.Mock).mock.calls.length > 0,
          { timeoutMs: 5000, intervalMs: 20, description: 'onConnection interceptor called' }
        );
        const context = (mockInterceptor.onConnection as jest.Mock).mock.calls[0][0] as ConnectionContext;
        expect(context.clientId).toBeDefined();
        expect(context.socket).toBeDefined();
    });

    test('should reject connection if onConnection throws', async () => {
        // Setup a server that rejects connections
        const rejectingInterceptor: IInterceptor = {
            name: 'RejectingInterceptor',
            onConnection: jest.fn().mockRejectedValue(new Error('Get out!'))
        };
        
        const rejectingServer = ServerFactory.create({
            port: 0,
            nodeId: 'reject-node',
            interceptors: [rejectingInterceptor]
        });
        await rejectingServer.ready();
        const rejectPort = rejectingServer.port;

        const ws = new WebSocket(`ws://localhost:${rejectPort}`);
        
        await new Promise<void>((resolve) => {
            ws.on('close', (code) => {
                expect(code).toBe(4000); // Connection Rejected code
                resolve();
            });
            ws.on('error', () => {}); // Ignore connection errors
        });

        await rejectingServer.shutdown();
    });

    // Note: Testing onBeforeOp requires full authentication flow or mocking internal methods.
    // Since ServerCoordinator is complex with Auth, we might want to unit test `processLocalOp` directly 
    // by casting to any and invoking private method, OR go through the full auth dance.
    // For this example, let's try accessing the private `processLocalOp` via cast to verify logic 
    // without setting up JWT infrastructure in the test.

    test('should execute interceptor pipeline in processLocalOp', async () => {
        const op: ServerOp = {
            mapName: 'test-map',
            key: 'k1',
            opType: 'PUT',
            record: { value: 'v1', timestamp: { millis: 1, counter: 0, nodeId: 'A' } }
        };

        // Mock onBeforeOp to modify the op
        const modifyingInterceptor: IInterceptor = {
            name: 'ModifyingInterceptor',
            onBeforeOp: jest.fn().mockImplementation(async (o) => ({
                ...o,
                record: { ...o.record, value: 'MODIFIED' }
            }))
        };

        const serverWithInterceptor = ServerFactory.create({
            port: 0,
            nodeId: 'test-node-2',
            storage: mockStorage,
            interceptors: [modifyingInterceptor]
        });
        
        await serverWithInterceptor.ready();

        // Access private method
        await (serverWithInterceptor as any).operationHandler.processLocalOp(op, false, 'client-1');

        expect(modifyingInterceptor.onBeforeOp).toHaveBeenCalledTimes(1);

        // Verify storage was called with modified value
        expect(mockStorage.store).toHaveBeenCalledWith(
            'test-map',
            'k1',
            expect.objectContaining({ value: 'MODIFIED' })
        );

        await serverWithInterceptor.shutdown();
    });

    test('should abort processLocalOp if interceptor throws', async () => {
        const op: ServerOp = {
            mapName: 'test-map',
            key: 'k1',
            opType: 'PUT',
            record: { value: 'v1', timestamp: { millis: 1, counter: 0, nodeId: 'A' } }
        };

        const throwingInterceptor: IInterceptor = {
            name: 'ThrowingInterceptor',
            onBeforeOp: jest.fn().mockRejectedValue(new Error('Block this!'))
        };

        const serverWithInterceptor = ServerFactory.create({
            port: 0,
            nodeId: 'test-node-3',
            storage: mockStorage,
            interceptors: [throwingInterceptor]
        });

        await serverWithInterceptor.ready();

        // Access private method and expect rejection
        await expect(
            (serverWithInterceptor as any).operationHandler.processLocalOp(op, false, 'client-1')
        ).rejects.toThrow('Block this!');

        // Verify storage was NOT called
        expect(mockStorage.store).not.toHaveBeenCalled();

        await serverWithInterceptor.shutdown();
    });

    test('should silently drop op if interceptor returns null', async () => {
        const op: ServerOp = {
            mapName: 'test-map',
            key: 'k2',
            opType: 'PUT',
            record: { value: 'v2', timestamp: { millis: 2, counter: 0, nodeId: 'A' } }
        };

        const droppingInterceptor: IInterceptor = {
            name: 'DroppingInterceptor',
            onBeforeOp: jest.fn().mockResolvedValue(null)
        };

        const serverWithInterceptor = ServerFactory.create({
            port: 0,
            nodeId: 'test-node-4',
            storage: mockStorage,
            interceptors: [droppingInterceptor]
        });

        await serverWithInterceptor.ready();

        // Access private method - should resolve without error
        await (serverWithInterceptor as any).operationHandler.processLocalOp(op, false, 'client-1');

        // Verify storage was NOT called
        expect(mockStorage.store).not.toHaveBeenCalled();

        await serverWithInterceptor.shutdown();
    });
});

