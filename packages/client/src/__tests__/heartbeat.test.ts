import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';

// --- Mock WebSocket ---
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.OPEN;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((error: any) => void) | null = null;
  sentMessages: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    // Simulate async connection
    setTimeout(() => {
      if (this.onopen) this.onopen();
    }, 0);
  }

  send(data: Uint8Array | string) {
    if (this.readyState !== MockWebSocket.OPEN) {
      throw new Error('WebSocket is not open');
    }
    if (data instanceof Uint8Array) {
      this.sentMessages.push(deserialize(data));
    } else {
      this.sentMessages.push(JSON.parse(data));
    }
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    if (this.onclose) this.onclose();
  }

  // Helper to simulate server message
  simulateMessage(message: any) {
    if (this.onmessage) {
      const data = serialize(message);
      const exactBuffer = data.slice().buffer as ArrayBuffer;
      this.onmessage({ data: exactBuffer });
    }
  }

  static reset() {
    MockWebSocket.instances = [];
  }

  static getLastInstance(): MockWebSocket | undefined {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

// Replace global WebSocket
(global as any).WebSocket = MockWebSocket;

// --- Mock Storage Adapter ---
function createMockStorageAdapter(): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
    getMeta: jest.fn().mockResolvedValue(undefined),
    setMeta: jest.fn().mockResolvedValue(undefined),
    batchPut: jest.fn().mockResolvedValue(undefined),
    appendOpLog: jest.fn().mockResolvedValue(1),
    getPendingOps: jest.fn().mockResolvedValue([]),
    markOpsSynced: jest.fn().mockResolvedValue(undefined),
    getAllKeys: jest.fn().mockResolvedValue([]),
  };
}

// --- Mock crypto.randomUUID ---
let uuidCounter = 0;
(global as any).crypto = {
  randomUUID: () => `test-uuid-${++uuidCounter}`,
};

/**
 * Helper to setup SyncEngine and authenticate
 * Returns the SyncEngine and the WebSocket instance
 */
async function setupAuthenticatedEngine(
  config: SyncEngineConfig,
  token: string = 'test-token'
): Promise<{ syncEngine: SyncEngine; ws: MockWebSocket }> {
  const syncEngine = new SyncEngine(config);
  syncEngine.setAuthToken(token);

  // Wait for connection (setTimeout with 0ms)
  await jest.advanceTimersByTimeAsync(1);

  const ws = MockWebSocket.getLastInstance()!;

  // Simulate AUTH_ACK - this will start heartbeat
  ws.simulateMessage({ type: 'AUTH_ACK' });

  // Process any immediate tasks (but don't advance time)
  await Promise.resolve();

  return { syncEngine, ws };
}

describe('Heartbeat', () => {
  describe('Client', () => {
    let syncEngine: SyncEngine;
    let mockStorage: jest.Mocked<IStorageAdapter>;
    let config: SyncEngineConfig;

    beforeEach(() => {
      jest.useFakeTimers();
      MockWebSocket.reset();
      uuidCounter = 0;

      mockStorage = createMockStorageAdapter();
      config = {
        nodeId: 'test-node',
        serverUrl: 'ws://localhost:8080',
        storageAdapter: mockStorage,
        reconnectInterval: 1000,
        heartbeat: {
          intervalMs: 5000,
          timeoutMs: 15000,
          enabled: true,
        },
      };
    });

    afterEach(() => {
      if (syncEngine) {
        syncEngine.close();
      }
      jest.useRealTimers();
      jest.clearAllMocks();
    });

    it('should send PING every intervalMs after connection', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      // Clear initial messages (AUTH)
      ws.sentMessages = [];

      // Advance by intervalMs - first PING should be sent
      await jest.advanceTimersByTimeAsync(5000);

      const pingMessages = ws.sentMessages.filter((m) => m.type === 'PING');
      expect(pingMessages).toHaveLength(1);
      expect(pingMessages[0].timestamp).toBeGreaterThan(0);
    });

    it('should update lastPongReceived on PONG', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      // Connection should be healthy after AUTH_ACK
      expect(syncEngine.isConnectionHealthy()).toBe(true);

      // Advance time to trigger PING
      await jest.advanceTimersByTimeAsync(5000);

      const pingMessage = ws.sentMessages.find((m) => m.type === 'PING');
      expect(pingMessage).toBeDefined();

      // Simulate PONG
      ws.simulateMessage({
        type: 'PONG',
        timestamp: pingMessage!.timestamp,
        serverTime: Date.now(),
      });
      await Promise.resolve();

      // Connection should still be healthy
      expect(syncEngine.isConnectionHealthy()).toBe(true);
    });

    it('should trigger reconnect if no PONG within timeoutMs', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      const initialInstanceCount = MockWebSocket.instances.length;

      // Advance time beyond timeout (15s) without receiving PONG
      // Need to advance past timeout + check interval
      await jest.advanceTimersByTimeAsync(20000);

      // WebSocket should have been closed due to timeout
      expect(ws.readyState).toBe(MockWebSocket.CLOSED);

      // Allow reconnect timer to fire (reconnectInterval = 1000ms)
      await jest.advanceTimersByTimeAsync(2000);

      // Should have reconnected
      expect(MockWebSocket.instances.length).toBeGreaterThan(initialInstanceCount);
    });

    it('should calculate round-trip time correctly', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      // Initially no RTT
      expect(syncEngine.getLastRoundTripTime()).toBeNull();

      // Send PING
      await jest.advanceTimersByTimeAsync(5000);

      const pingMessage = ws.sentMessages.find((m) => m.type === 'PING');
      expect(pingMessage).toBeDefined();

      // Simulate PONG after 50ms
      await jest.advanceTimersByTimeAsync(50);
      ws.simulateMessage({
        type: 'PONG',
        timestamp: pingMessage!.timestamp,
        serverTime: Date.now(),
      });
      await Promise.resolve();

      // RTT should be approximately 50ms
      const rtt = syncEngine.getLastRoundTripTime();
      expect(rtt).not.toBeNull();
      expect(rtt).toBeGreaterThanOrEqual(0);
    });

    it('should stop heartbeat on disconnect', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      // Heartbeat should be running - advance time to trigger PING
      ws.sentMessages = [];
      await jest.advanceTimersByTimeAsync(5000);
      expect(ws.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(1);

      // Close connection
      ws.close();
      await Promise.resolve();

      // Wait for reconnect timer
      await jest.advanceTimersByTimeAsync(1000);

      const newWs = MockWebSocket.getLastInstance()!;
      newWs.sentMessages = [];

      // Advance time - PING should NOT be sent because not authenticated yet
      await jest.advanceTimersByTimeAsync(5000);

      expect(newWs.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(0);
    });

    it('should not send PING if heartbeat disabled', async () => {
      config.heartbeat = {
        intervalMs: 5000,
        timeoutMs: 15000,
        enabled: false,
      };

      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      ws.sentMessages = [];

      // Advance by multiple intervals
      await jest.advanceTimersByTimeAsync(20000);

      // No PING messages should be sent
      const pingMessages = ws.sentMessages.filter((m) => m.type === 'PING');
      expect(pingMessages).toHaveLength(0);
    });

    it('should report connection as healthy when heartbeat is disabled', async () => {
      config.heartbeat = {
        intervalMs: 5000,
        timeoutMs: 15000,
        enabled: false,
      };

      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;

      // Should be healthy even without PONGs because heartbeat is disabled
      expect(syncEngine.isConnectionHealthy()).toBe(true);
    });

    it('should report connection as unhealthy when not online', async () => {
      syncEngine = new SyncEngine(config);
      await jest.advanceTimersByTimeAsync(1);

      const ws = MockWebSocket.getLastInstance()!;
      ws.close();
      await Promise.resolve();

      expect(syncEngine.isConnectionHealthy()).toBe(false);
    });

    it('should report connection as unhealthy when not authenticated', async () => {
      syncEngine = new SyncEngine(config);
      await jest.advanceTimersByTimeAsync(1);

      // Connected but not authenticated
      expect(syncEngine.isConnectionHealthy()).toBe(false);
    });

    it('should use default heartbeat config when not specified', async () => {
      const configWithoutHeartbeat: SyncEngineConfig = {
        nodeId: 'test-node',
        serverUrl: 'ws://localhost:8080',
        storageAdapter: mockStorage,
      };

      const result = await setupAuthenticatedEngine(configWithoutHeartbeat);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      ws.sentMessages = [];

      // Default interval is 5000ms
      await jest.advanceTimersByTimeAsync(5000);

      const pingMessages = ws.sentMessages.filter((m) => m.type === 'PING');
      expect(pingMessages).toHaveLength(1);
    });

    it('should send multiple PINGs over time', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;
      const ws = result.ws;

      ws.sentMessages = [];

      // Simulate PONG responses to keep connection alive
      for (let i = 0; i < 3; i++) {
        await jest.advanceTimersByTimeAsync(5000);

        // Find the latest PING and respond with PONG
        const pings = ws.sentMessages.filter((m) => m.type === 'PING');
        const lastPing = pings[pings.length - 1];
        if (lastPing) {
          ws.simulateMessage({
            type: 'PONG',
            timestamp: lastPing.timestamp,
            serverTime: Date.now(),
          });
          await Promise.resolve();
        }
      }

      const pingMessages = ws.sentMessages.filter((m) => m.type === 'PING');
      expect(pingMessages).toHaveLength(3);
    });

    it('should report connection as unhealthy when PONG not received within timeout', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;

      // Initially healthy
      expect(syncEngine.isConnectionHealthy()).toBe(true);

      // Advance time beyond timeout (15s) - connection should become unhealthy
      // Note: At exactly 15s, the check runs at 15s interval, so we need to go past that
      await jest.advanceTimersByTimeAsync(16000);

      // At this point, the connection should be closed and isOnline should be false
      // due to the heartbeat timeout
      expect(syncEngine.isConnectionHealthy()).toBe(false);
    });
  });
});
