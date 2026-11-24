import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { heartbeatExpired } from '../sync/WebSocketManager';

// --- Mock WebSocket ---
class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.OPEN;
  binaryType: string = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
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
    if (this.onclose) this.onclose({ code: 1000, reason: 'Normal closure' });
  }

  // Helper to simulate server message
  simulateMessage(message: any) {
    if (this.onmessage) {
      const data = serialize(message);
      // Create a proper ArrayBuffer copy (msgpackr returns Buffer which shares memory)
      const exactBuffer = new Uint8Array(data).buffer;
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
    deleteOp: jest.fn().mockResolvedValue(undefined),
    commitWrite: jest.fn().mockResolvedValue(1),
    getAllKeys: jest.fn().mockResolvedValue([]),
    getAllMetaKeys: jest.fn().mockResolvedValue([]),
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
  token: string = 'test-token',
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
        connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
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

    it('should close connection on heartbeat timeout', async () => {
      const result = await setupAuthenticatedEngine(config);
      syncEngine = result.syncEngine;

      // Advance time beyond timeout (15s) without receiving PONG
      // Need to advance past timeout + check interval
      await jest.advanceTimersByTimeAsync(20000);

      // Connection should be closed and marked unhealthy due to timeout
      expect(syncEngine.isConnectionHealthy()).toBe(false);
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
        connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
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

  // TG-WS-003: any inbound message is liveness, and the heartbeat expires only
  // while a PING is outstanding and more than timeoutMs has passed since the
  // last inbound message.
  describe('Liveness (TG-WS-003)', () => {
    const INTERVAL = 5000;
    const TIMEOUT = 15000;
    let syncEngine: SyncEngine;
    let provider: SingleServerProvider;
    let forceReconnect: jest.SpyInstance;
    let config: SyncEngineConfig;

    beforeEach(() => {
      jest.useFakeTimers();
      MockWebSocket.reset();
      uuidCounter = 0;
      provider = new SingleServerProvider({ url: 'ws://localhost:8080' });
      forceReconnect = jest.spyOn(provider, 'forceReconnect');
      config = {
        nodeId: 'test-node',
        connectionProvider: provider,
        storageAdapter: createMockStorageAdapter(),
        reconnectInterval: 1000,
        heartbeat: { intervalMs: INTERVAL, timeoutMs: TIMEOUT, enabled: true },
      };
    });

    afterEach(() => {
      syncEngine?.close();
      jest.restoreAllMocks();
      jest.useRealTimers();
    });

    /** Moves the clock by `ms` without firing timers, so the next tick comes that much later. */
    function suspend(ms: number): void {
      jest.setSystemTime(Date.now() + ms);
    }

    describe('T1: predicate boundaries', () => {
      it('expires only past timeoutMs, and never without an outstanding PING', () => {
        expect(heartbeatExpired(15000, 0, true, TIMEOUT)).toBe(false);
        expect(heartbeatExpired(15001, 0, true, TIMEOUT)).toBe(true);
        expect(heartbeatExpired(15001, 0, false, TIMEOUT)).toBe(false);
        expect(heartbeatExpired(1e12, 0, false, TIMEOUT)).toBe(false);
      });

      it('reads healthy at exactly timeoutMs since the last inbound message', async () => {
        ({ syncEngine } = await setupAuthenticatedEngine(config));
        await jest.advanceTimersByTimeAsync(TIMEOUT);
        expect(forceReconnect).not.toHaveBeenCalled();
        expect(syncEngine.isConnectionHealthy()).toBe(true);
        await jest.advanceTimersByTimeAsync(1);
        expect(syncEngine.isConnectionHealthy()).toBe(false);
      });

      it('a message and then a tick in the same millisecond leave the PING outstanding', async () => {
        let ws: MockWebSocket;
        ({ syncEngine, ws } = await setupAuthenticatedEngine(config));
        await jest.advanceTimersByTimeAsync(INTERVAL - 1);
        const due = Date.now() + 1;

        // Message and tick share one Date.now(): the message first, then the tick.
        const now = jest.spyOn(Date, 'now').mockReturnValue(due);
        ws.simulateMessage({ type: 'QUERY_UPDATE', payload: {} });
        await jest.advanceTimersByTimeAsync(1);
        now.mockRestore();
        expect(ws.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(1);
        expect(forceReconnect).not.toHaveBeenCalled();

        // Silence, and the next tick 60 s after the message.
        suspend(60_000 - INTERVAL);
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(forceReconnect).toHaveBeenCalledTimes(1);
      });

      it('a tick and then a message in the same millisecond clear the PING', async () => {
        let ws: MockWebSocket;
        ({ syncEngine, ws } = await setupAuthenticatedEngine(config));
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(ws.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(1);
        ws.simulateMessage({ type: 'QUERY_UPDATE', payload: {} });

        // The late tick only pings: nothing was outstanding.
        suspend(60_000 - INTERVAL);
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(forceReconnect).not.toHaveBeenCalled();

        // Its own PING goes unanswered, so the tick after it reconnects.
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(forceReconnect).toHaveBeenCalledTimes(1);
      });
    });

    describe('T2: throttled ticks', () => {
      it('does not reconnect at a late tick when the last PING was answered', async () => {
        let ws: MockWebSocket;
        ({ syncEngine, ws } = await setupAuthenticatedEngine(config));
        await jest.advanceTimersByTimeAsync(INTERVAL);
        const ping = ws.sentMessages.find((m) => m.type === 'PING');
        expect(ping).toBeDefined();
        await jest.advanceTimersByTimeAsync(50);
        ws.simulateMessage({ type: 'PONG', timestamp: ping.timestamp, serverTime: Date.now() });

        suspend(60_000 - INTERVAL);
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(forceReconnect).not.toHaveBeenCalled();
        expect(ws.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(2);
      });

      it('reconnects at the late tick when the last PING was not answered', async () => {
        ({ syncEngine } = await setupAuthenticatedEngine(config));
        await jest.advanceTimersByTimeAsync(INTERVAL);

        suspend(60_000 - INTERVAL);
        await jest.advanceTimersByTimeAsync(INTERVAL);
        expect(forceReconnect).toHaveBeenCalledTimes(1);
      });
    });

    describe('T3: a stream of CHUNKs with no PONG', () => {
      it('keeps the connection for 60 s, then reconnects at the first tick more than timeoutMs after the last message', async () => {
        let ws: MockWebSocket;
        ({ syncEngine, ws } = await setupAuthenticatedEngine(config));
        const inner = serialize({ type: 'QUERY_UPDATE', payload: { pad: 'x'.repeat(600) } });
        const parts = 30;
        const size = Math.ceil(inner.length / parts);

        for (let i = 0; i < parts; i++) {
          await jest.advanceTimersByTimeAsync(2000);
          ws.simulateMessage({
            type: 'CHUNK',
            last: i === parts - 1,
            data: inner.slice(i * size, (i + 1) * size),
          });
        }
        expect(forceReconnect).not.toHaveBeenCalled();

        // The last message came at 60 s, together with a tick that ran before it.
        // Ticks follow at 65, 70, 75 (exactly 15 s: not yet) and 80 s.
        await jest.advanceTimersByTimeAsync(TIMEOUT + INTERVAL - 1);
        expect(forceReconnect).not.toHaveBeenCalled();
        await jest.advanceTimersByTimeAsync(1);
        expect(forceReconnect).toHaveBeenCalledTimes(1);
      });
    });

    describe('T9: a socket that can no longer send', () => {
      it('still expires: every tick leaves a PING outstanding, sent or not', async () => {
        let ws: MockWebSocket;
        ({ syncEngine, ws } = await setupAuthenticatedEngine(config));
        ws.sentMessages = [];
        // CLOSING with a stalled closing handshake: not OPEN, and no close event.
        ws.readyState = 2;
        expect(provider.isConnected()).toBe(false);

        await jest.advanceTimersByTimeAsync(TIMEOUT + INTERVAL - 1);
        expect(forceReconnect).not.toHaveBeenCalled();
        await jest.advanceTimersByTimeAsync(1);
        expect(forceReconnect).toHaveBeenCalledTimes(1);
        expect(ws.sentMessages.filter((m) => m.type === 'PING')).toHaveLength(0);
      });
    });
  });
});
