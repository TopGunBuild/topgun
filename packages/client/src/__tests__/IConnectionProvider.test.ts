/**
 * Tests for IConnectionProvider abstraction (Phase 4.5 Task 02)
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

    test('should accept connectionProvider in config', () => {
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
      engine.close();
      provider.close();
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

      expect(() => new SyncEngine({
        nodeId: 'test-node',
        storageAdapter: mockStorage,
      } as any)).toThrow('SyncEngine requires connectionProvider');
    });
  });
});
