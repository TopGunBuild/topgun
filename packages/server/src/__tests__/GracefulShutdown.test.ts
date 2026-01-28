import { ServerCoordinator, ServerFactory } from '../';
import { IServerStorage } from '../storage/IServerStorage';
import { WebSocket } from 'ws';
import { deserialize } from '@topgunbuild/core';

describe('ServerCoordinator Graceful Shutdown', () => {
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

    server = ServerFactory.create({
      port: 0,
      nodeId: 'test-server',
      storage: mockStorage,
    });

    await server.ready();
    actualPort = server.port;
  });

  afterEach(async () => {
    // Ensure server is stopped if test fails
    try {
      await server.shutdown();
    } catch (e) {
      // Ignore if already stopped
    }
  });

  test('should close HTTP and WebSocket servers', async () => {
    await server.shutdown();

    // Verify we can't connect anymore
    const connectPromise = new Promise((resolve, reject) => {
      const ws = new WebSocket(`ws://localhost:${actualPort}`);
      ws.on('open', () => resolve(true));
      ws.on('error', (err: Error) => reject(err));
    });

    await expect(connectPromise).rejects.toThrow();
  });

  test('should close storage connection', async () => {
    await server.shutdown();
    expect(mockStorage.close).toHaveBeenCalled();
  });

  test('should notify connected clients before closing', async () => {
    // 1. Connect Client
    const ws = new WebSocket(`ws://localhost:${actualPort}`);

    const messages: any[] = [];
    const closedPromise = new Promise<void>((resolve) => {
        ws.on('close', () => resolve());
    });

    await new Promise<void>((resolve) => {
        ws.on('open', () => resolve());
    });

    ws.on('message', (data: any) => {
        const msg = deserialize(data as Buffer);
        messages.push(msg);
    });

    // 2. Shutdown
    await server.shutdown();

    // 3. Wait for close
    await closedPromise;

    // 4. Verify
    const shutdownMsg = messages.find(m => m.type === 'SHUTDOWN_PENDING');
    expect(shutdownMsg).toBeDefined();
    expect(shutdownMsg.retryAfter).toBe(5000);
  });
});
