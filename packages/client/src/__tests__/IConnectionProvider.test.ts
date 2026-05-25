/**
 * Tests for IConnectionProvider abstraction
 */
import { SingleServerProvider } from '../connection/SingleServerProvider';
import type { IConnectionProvider, ConnectionProviderEvent } from '../types';

describe('IConnectionProvider', () => {
  describe('SingleServerProvider', () => {
    let provider: SingleServerProvider;

    beforeEach(() => {
      provider = new SingleServerProvider({
        url: 'ws://localhost:9999',
        maxReconnectAttempts: 2,
        reconnectDelayMs: 100,
      });
    });

    afterEach(async () => {
      await provider.close();
    });

    test('should implement IConnectionProvider interface', () => {
      // Type check - SingleServerProvider should be assignable to IConnectionProvider
      const iface: IConnectionProvider = provider;
      expect(iface).toBeDefined();
      expect(typeof iface.connect).toBe('function');
      expect(typeof iface.getConnection).toBe('function');
      expect(typeof iface.getAnyConnection).toBe('function');
      expect(typeof iface.isConnected).toBe('function');
      expect(typeof iface.getConnectedNodes).toBe('function');
      expect(typeof iface.on).toBe('function');
      expect(typeof iface.off).toBe('function');
      expect(typeof iface.send).toBe('function');
      expect(typeof iface.close).toBe('function');
    });

    test('should return false for isConnected() before connect', () => {
      expect(provider.isConnected()).toBe(false);
    });

    test('should return empty array for getConnectedNodes() before connect', () => {
      expect(provider.getConnectedNodes()).toEqual([]);
    });

    test('should throw when getConnection() called before connect', () => {
      expect(() => provider.getConnection('key')).toThrow('Not connected');
    });

    test('should throw when getAnyConnection() called before connect', () => {
      expect(() => provider.getAnyConnection()).toThrow('Not connected');
    });

    test('should throw when send() called before connect', () => {
      expect(() => provider.send(new Uint8Array([1, 2, 3]))).toThrow('Not connected');
    });

    test('should support event subscription and unsubscription', () => {
      const handler = jest.fn();

      provider.on('connected', handler);
      provider.off('connected', handler);

      // Should not throw
      expect(true).toBe(true);
    });

    test('should return correct URL', () => {
      expect(provider.getUrl()).toBe('ws://localhost:9999');
    });

    test('should support resetReconnectAttempts()', () => {
      expect(provider.getReconnectAttempts()).toBe(0);
      provider.resetReconnectAttempts();
      expect(provider.getReconnectAttempts()).toBe(0);
    });

    test('should not throw on close() even if not connected', async () => {
      await expect(provider.close()).resolves.not.toThrow();
    });
  });

  describe('SyncEngine with IConnectionProvider', () => {
    // Import SyncEngine only for this test block
    const { SyncEngine } = require('../SyncEngine');

    // Install a MockWebSocket for this block. The test doesn't exercise real
    // network behavior — it just verifies SyncEngine accepts the provider
    // config. Without a mock, the real undici WebSocket would dial out and
    // leave SingleServerProvider's 5s connection-timeout pending past the
    // test, keeping Jest's worker alive without --forceExit.
    const originalWebSocket = (globalThis as any).WebSocket;
    beforeAll(() => {
      (globalThis as any).WebSocket = class MockWebSocket {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;
        readyState = 1;
        binaryType = 'arraybuffer';
        onopen: (() => void) | null = null;
        onclose: (() => void) | null = null;
        onmessage: ((event: { data: any }) => void) | null = null;
        onerror: ((error: any) => void) | null = null;
        send = jest.fn();
        close = jest.fn();
        constructor(public url: string) {
          // queueMicrotask, not setTimeout(0) — see backpressure.test.ts
          // for the rationale: avoids a timer-handle leak and lets the SUT's
          // onopen wrapper clear its 5s connection-timeout same-tick.
          queueMicrotask(() => {
            if (this.onopen) this.onopen();
          });
        }
      };
    });

    afterAll(() => {
      (globalThis as any).WebSocket = originalWebSocket;
    });

    test('should accept connectionProvider in config', async () => {
      const mockStorage = {
        put: jest.fn(),
        get: jest.fn(),
        remove: jest.fn(),
        getAllKeys: jest.fn().mockResolvedValue([]),
        appendOpLog: jest.fn().mockResolvedValue(1),
        getPendingOps: jest.fn().mockResolvedValue([]),
        markOpsSynced: jest.fn(),
        getMeta: jest.fn().mockResolvedValue(null),
        setMeta: jest.fn(),
      };

      const provider = new SingleServerProvider({ url: 'ws://localhost:8080' });

      const engine = new SyncEngine({
        nodeId: 'test-node',
        connectionProvider: provider,
        storageAdapter: mockStorage,
      });

      expect(engine).toBeDefined();
      // Flush the queueMicrotask that MockWebSocket schedules — it triggers
      // SingleServerProvider's onopen wrapper which clears the 5s connection
      // timeout (SingleServerProvider.ts:100). Without this flush the timeout
      // leaks past the end of this sync test as an open handle.
      await Promise.resolve();
      engine.close();
      await provider.close();
    });

    test('should throw if connectionProvider not provided', () => {
      const mockStorage = {
        put: jest.fn(),
        get: jest.fn(),
        remove: jest.fn(),
        getAllKeys: jest.fn().mockResolvedValue([]),
        appendOpLog: jest.fn().mockResolvedValue(1),
        getPendingOps: jest.fn().mockResolvedValue([]),
        markOpsSynced: jest.fn(),
        getMeta: jest.fn().mockResolvedValue(null),
        setMeta: jest.fn(),
      };

      expect(
        () =>
          new SyncEngine({
            nodeId: 'test-node',
            storageAdapter: mockStorage,
          } as any),
      ).toThrow('SyncEngine requires connectionProvider');
    });
  });
});
