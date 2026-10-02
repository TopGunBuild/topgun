import { SyncEngine, SyncEngineConfig, OpLogEntry } from '../SyncEngine';
import { IStorageAdapter, OpLogEntry as StoredOpLogEntry } from '../IStorageAdapter';
import { serialize, deserialize, LWWMap, ORMap, HLC } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { logger } from '../utils/logger';

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

  // Helper to simulate JSON message
  simulateJsonMessage(message: any) {
    if (this.onmessage) {
      this.onmessage({ data: JSON.stringify(message) });
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
    // Default to an already-migrated device: the one-time OR-Map marker backfill
    // (SyncEngine.backfillLegacyOrMapMarkers) is gated on this flag, so unrelated
    // tests are not perturbed by its startup keyspace scan. Backfill-specific tests
    // override getMeta to leave the flag unset.
    getMeta: jest
      .fn()
      .mockImplementation((key: string) =>
        Promise.resolve(key === '__sys__:ormapBackfillDone' ? true : undefined),
      ),
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

describe('SyncEngine', () => {
  let syncEngine: SyncEngine | undefined;
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
      // Disable heartbeat for these tests to prevent fake timer conflicts
      heartbeat: { enabled: false },
    };
  });

  afterEach(() => {
    // Dispose the engine before switching back to real timers so that the
    // synchronous portion of teardown (clearTimeout on SingleServerProvider's
    // reconnectTimer, removeEventListener on online/offline) runs while the
    // fake-timer scheduler is still active. Otherwise the timer scheduled in
    // scheduleReconnect() (called from the WebSocket onclose handler) leaks
    // into the real event loop and keeps Jest's worker alive past the last
    // expect(), which historically forced --forceExit as a bandaid.
    if (syncEngine) {
      syncEngine.close();
      syncEngine = undefined;
    }
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  describe('Initialization', () => {
    test('should create WebSocket connection on construction', () => {
      syncEngine = new SyncEngine(config);

      expect(MockWebSocket.instances).toHaveLength(1);
      expect(MockWebSocket.instances[0].url).toBe('ws://localhost:8080');
    });

    test('should load pending ops from storage on init', async () => {
      const pendingOps = [
        {
          id: '1',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      expect(mockStorage.getPendingOps).toHaveBeenCalled();
    });

    test('should load last sync timestamp from storage', async () => {
      mockStorage.getMeta.mockResolvedValue(12345);

      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      expect(mockStorage.getMeta).toHaveBeenCalledWith('lastSyncTimestamp');
    });

    test('should set binaryType to arraybuffer', () => {
      syncEngine = new SyncEngine(config);

      const ws = MockWebSocket.getLastInstance();
      expect(ws?.binaryType).toBe('arraybuffer');
    });
  });

  describe('Connection lifecycle', () => {
    test('should set isOnline=true when WebSocket opens', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      expect(ws).toBeDefined();
      // Connection is established (onopen was called)
    });

    test('should send AUTH when token is available on connect', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      const authMessage = ws?.sentMessages.find((m) => m.type === 'AUTH');
      expect(authMessage).toBeDefined();
      expect(authMessage?.token).toBe('test-token');
    });

    test('presents a DEVICE_HELLO (not an empty-token AUTH) when no token is configured (token-less present-or-mint)', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      // Token-less clients present device identity on a dedicated DEVICE_HELLO — never an
      // empty-token AUTH (a real JWT server would AUTH_FAIL + disconnect that). No JWT is
      // sent and no device credential exists yet.
      expect(ws?.sentMessages.find((m) => m.type === 'AUTH')).toBeUndefined();
      const hello = ws?.sentMessages.find((m) => m.type === 'DEVICE_HELLO');
      expect(hello).toBeDefined();
      expect(hello?.deviceToken).toBeUndefined();
    });

    test('should schedule reconnect after disconnect', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      ws?.close();

      expect(MockWebSocket.instances).toHaveLength(1);

      // Advance timer for reconnect
      jest.advanceTimersByTime(1000);
      await jest.runAllTimersAsync();

      expect(MockWebSocket.instances).toHaveLength(2);
    });

    test('should reconnect immediately when token is set during backoff', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      ws?.close();

      // Before reconnect timer fires, set token
      jest.advanceTimersByTime(500); // Half of reconnect interval
      syncEngine.setAuthToken('new-token');
      await jest.runAllTimersAsync();

      // Should reconnect immediately
      expect(MockWebSocket.instances).toHaveLength(2);
    });
  });

  describe('Authentication', () => {
    test('should respond to AUTH_REQUIRED by sending AUTH', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('my-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = []; // Clear initial auth

      ws.simulateMessage({ type: 'AUTH_REQUIRED' });
      await jest.runAllTimersAsync();

      const authMessage = ws.sentMessages.find((m) => m.type === 'AUTH');
      expect(authMessage).toBeDefined();
    });

    test('should set isAuthenticated=true on AUTH_ACK', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('my-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // Verify by checking that pending operations are synced
      // (this happens after AUTH_ACK)
    });

    test('should clear token on AUTH_FAIL', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('invalid-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_FAIL', error: 'Invalid token' });
      await jest.runAllTimersAsync();

      // Token should be cleared, won't send on next AUTH_REQUIRED
      ws.sentMessages = [];
      ws.simulateMessage({ type: 'AUTH_REQUIRED' });
      await jest.runAllTimersAsync();

      const authMessage = ws.sentMessages.find((m) => m.type === 'AUTH');
      expect(authMessage).toBeUndefined();
    });

    test('should support token provider', async () => {
      const tokenProvider = jest.fn().mockResolvedValue('provider-token');

      syncEngine = new SyncEngine(config);
      syncEngine.setTokenProvider(tokenProvider);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      const authMessage = ws.sentMessages.find((m) => m.type === 'AUTH');

      expect(tokenProvider).toHaveBeenCalled();
      expect(authMessage?.token).toBe('provider-token');
    });
  });

  describe('Operation recording and syncing', () => {
    test('should record operation to opLog and storage', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'test-node' };
      const record = { value: { name: 'Alice' }, timestamp };

      await syncEngine.recordOperation('users', 'PUT', 'user1', { record, timestamp });

      expect(mockStorage.appendOpLog).toHaveBeenCalledWith(
        expect.objectContaining({
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
        }),
      );
    });

    test('should sync pending operations after AUTH_ACK', async () => {
      const pendingOps = [
        {
          id: '1',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = [];

      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      const opBatch = ws.sentMessages.find((m) => m.type === 'OP_BATCH');
      expect(opBatch).toBeDefined();
      expect(opBatch?.payload?.ops).toHaveLength(1);
    });

    test('should mark operations as synced on OP_ACK', async () => {
      // Setup pending ops so there are items in opLog to mark as synced
      const pendingOps = [
        {
          id: '3',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
        {
          id: '5',
          mapName: 'users',
          opType: 'PUT',
          key: 'user2',
          synced: false,
          timestamp: { millis: 1001, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '5', achievedLevel: 'APPLIED' } });
      await jest.runAllTimersAsync();

      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(5);
    });
  });

  describe('Query subscriptions', () => {
    test('should send QUERY_SUB after AUTH_ACK', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const mockQuery = {
        id: 'query-1',
        getMapName: () => 'users',
        getFilter: () => ({ where: { active: true } }),
        onResult: jest.fn(),
        onUpdate: jest.fn(),
      };

      syncEngine.subscribeToQuery(mockQuery as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = [];
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      const querySub = ws.sentMessages.find((m) => m.type === 'QUERY_SUB');
      expect(querySub).toBeDefined();
      expect(querySub?.payload?.queryId).toBe('query-1');
      expect(querySub?.payload?.mapName).toBe('users');
    });

    test('should handle QUERY_RESP message with server source', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const mockQuery = {
        id: 'query-1',
        getMapName: () => 'users',
        getFilter: () => ({}),
        onResult: jest.fn(),
        onUpdate: jest.fn(),
        updatePaginationInfo: jest.fn(),
      };

      syncEngine.subscribeToQuery(mockQuery as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'QUERY_RESP',
        payload: {
          queryId: 'query-1',
          results: [{ key: 'user1', value: { name: 'Alice' } }],
          nextCursor: 'cursor123',
          hasMore: true,
          cursorStatus: 'none',
        },
      });
      await jest.runAllTimersAsync();

      // Verify onResult is called with 'server' source parameter and optional merkleRootHash
      expect(mockQuery.onResult).toHaveBeenCalledWith(
        [{ key: 'user1', value: { name: 'Alice' } }],
        'server',
        undefined,
      );

      // Verify pagination info is updated
      expect(mockQuery.updatePaginationInfo).toHaveBeenCalledWith({
        nextCursor: 'cursor123',
        hasMore: true,
        cursorStatus: 'none',
      });
    });

    test('should handle QUERY_UPDATE message', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const mockQuery = {
        id: 'query-1',
        getMapName: () => 'users',
        getFilter: () => ({}),
        onResult: jest.fn(),
        onUpdate: jest.fn(),
      };

      syncEngine.subscribeToQuery(mockQuery as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'QUERY_UPDATE',
        payload: {
          queryId: 'query-1',
          key: 'user1',
          value: { name: 'Bob' },
          type: 'UPDATE',
        },
      });
      await jest.runAllTimersAsync();

      expect(mockQuery.onUpdate).toHaveBeenCalledWith('user1', { name: 'Bob' });
    });

    test('should handle QUERY_UPDATE with LEAVE type', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const mockQuery = {
        id: 'query-1',
        getMapName: () => 'users',
        getFilter: () => ({}),
        onResult: jest.fn(),
        onUpdate: jest.fn(),
      };

      syncEngine.subscribeToQuery(mockQuery as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'QUERY_UPDATE',
        payload: {
          queryId: 'query-1',
          key: 'user1',
          value: null,
          changeType: 'LEAVE',
        },
      });
      await jest.runAllTimersAsync();

      expect(mockQuery.onUpdate).toHaveBeenCalledWith('user1', null);
    });

    test('should send QUERY_UNSUB when unsubscribing', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      ws.sentMessages = [];
      syncEngine.unsubscribeFromQuery('query-1');

      const queryUnsub = ws.sentMessages.find((m) => m.type === 'QUERY_UNSUB');
      expect(queryUnsub).toBeDefined();
      expect(queryUnsub?.payload?.queryId).toBe('query-1');
    });
  });

  describe('Topic pub/sub', () => {
    test('should send TOPIC_SUB when subscribing to topic', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();
      ws.sentMessages = [];

      const mockHandle = { onMessage: jest.fn() };
      syncEngine.subscribeToTopic('chat-room', mockHandle as any);

      const topicSub = ws.sentMessages.find((m) => m.type === 'TOPIC_SUB');
      expect(topicSub).toBeDefined();
      expect(topicSub?.payload?.topic).toBe('chat-room');
    });

    test('should handle TOPIC_MESSAGE', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const mockHandle = { onMessage: jest.fn() };
      syncEngine.subscribeToTopic('chat-room', mockHandle as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'TOPIC_MESSAGE',
        payload: {
          topic: 'chat-room',
          data: { text: 'Hello!' },
          publisherId: 'node-2',
          timestamp: 123456,
        },
      });
      await jest.runAllTimersAsync();

      expect(mockHandle.onMessage).toHaveBeenCalledWith(
        { text: 'Hello!' },
        { publisherId: 'node-2', timestamp: 123456 },
      );
    });

    test('should send TOPIC_PUB when publishing', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();
      ws.sentMessages = [];

      syncEngine.publishTopic('chat-room', { text: 'Hi!' });

      const topicPub = ws.sentMessages.find((m) => m.type === 'TOPIC_PUB');
      expect(topicPub).toBeDefined();
      expect(topicPub?.payload?.topic).toBe('chat-room');
      expect(topicPub?.payload?.data).toEqual({ text: 'Hi!' });
    });

    test('should NOT publish when offline', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.close();

      ws.sentMessages = [];
      syncEngine.publishTopic('chat-room', { text: 'Hi!' });

      expect(ws.sentMessages).toHaveLength(0);
    });

    test('should send TOPIC_UNSUB when unsubscribing', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();
      ws.sentMessages = [];

      syncEngine.unsubscribeFromTopic('chat-room');

      const topicUnsub = ws.sentMessages.find((m) => m.type === 'TOPIC_UNSUB');
      expect(topicUnsub).toBeDefined();
      expect(topicUnsub?.payload?.topic).toBe('chat-room');
    });
  });

  describe('Distributed locks', () => {
    test('should send LOCK_REQUEST', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();
      ws.sentMessages = [];

      const lockPromise = syncEngine.requestLock('my-lock', 'req-1', 10000);

      const lockReq = ws.sentMessages.find((m) => m.type === 'LOCK_REQUEST');
      expect(lockReq).toBeDefined();
      expect(lockReq?.payload?.name).toBe('my-lock');
      expect(lockReq?.payload?.ttl).toBe(10000);

      // Simulate grant
      ws.simulateMessage({
        type: 'LOCK_GRANTED',
        payload: { requestId: 'req-1', fencingToken: 42 },
      });
      await jest.runAllTimersAsync();

      const result = await lockPromise;
      expect(result.fencingToken).toBe(42);
    });

    test('should handle lock request timeout', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // ttl=5000 → response timeout = max(5000 + 5000, 5000) = 10000ms
      const lockPromise = syncEngine.requestLock('my-lock', 'req-1', 5000);

      // Advance time beyond the TTL-coordinated 10s timeout (5s ttl + 5s grace)
      jest.advanceTimersByTime(10001);

      await expect(lockPromise).rejects.toThrow('Lock request timed out');
    });

    test('should use TTL-coordinated response timeout for long-TTL lock request', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // ttl=60000 → response timeout = max(60000 + 5000, 5000) = 65000ms
      const lockPromise = syncEngine.requestLock('my-lock', 'req-long', 60000);

      // At 30s the lock should NOT have timed out (old hardcoded 30s bug would fail here)
      jest.advanceTimersByTime(30001);
      const raceResult = await Promise.race([
        lockPromise.then(() => 'resolved').catch(() => 'rejected'),
        Promise.resolve('pending'),
      ]);
      expect(raceResult).toBe('pending');

      // Advance to beyond 65s — now it should reject
      jest.advanceTimersByTime(35000); // total: ~65001ms
      await expect(lockPromise).rejects.toThrow('Lock request timed out');
    });

    test('should send LOCK_RELEASE', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();
      ws.sentMessages = [];

      const releasePromise = syncEngine.releaseLock('my-lock', 'req-2', 42);

      const lockRelease = ws.sentMessages.find((m) => m.type === 'LOCK_RELEASE');
      expect(lockRelease).toBeDefined();
      expect(lockRelease?.payload?.name).toBe('my-lock');
      expect(lockRelease?.payload?.fencingToken).toBe(42);

      // Simulate release ack
      ws.simulateMessage({
        type: 'LOCK_RELEASED',
        payload: { requestId: 'req-2', success: true },
      });
      await jest.runAllTimersAsync();

      const result = await releasePromise;
      expect(result).toBe(true);
    });

    test('should reject lock request when not connected', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.close();

      await expect(syncEngine.requestLock('my-lock', 'req-1', 5000)).rejects.toThrow(
        'Not connected or authenticated',
      );
    });
  });

  describe('Map registration and SERVER_EVENT handling', () => {
    test('should register map', () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);

      syncEngine.registerMap('users', lwwMap);
      // No error means success
    });

    test('should handle SERVER_EVENT for LWWMap', async () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      syncEngine.registerMap('users', lwwMap);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      const timestamp = { millis: Date.now(), counter: 1, nodeId: 'remote-node' };

      ws.simulateMessage({
        type: 'SERVER_EVENT',
        payload: {
          mapName: 'users',
          eventType: 'PUT',
          key: 'user1',
          record: { value: { name: 'Alice' }, timestamp },
        },
      });
      await jest.runAllTimersAsync();

      expect(lwwMap.get('user1')).toEqual({ name: 'Alice' });
      expect(mockStorage.put).toHaveBeenCalledWith(
        'users:user1',
        expect.objectContaining({ value: { name: 'Alice' } }),
      );
    });

    test('adopts a rejected echo of our own write so disk matches memory (F8)', async () => {
      // The client writes optimistically with a timestamp ahead of the server.
      // The server re-stamps with a lower (arrival-order) timestamp and echoes
      // the same value back. The echo loses LWW, but the old code persisted it
      // unconditionally — disk got the server ts while memory kept the client ts
      // (Merkle skew). The fix adopts the server record so memory AND disk land
      // on the server timestamp.
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      syncEngine.registerMap('users', lwwMap);
      await jest.runAllTimersAsync();

      const value = { name: 'Alice' };
      // Optimistic local write with a timestamp far ahead of the server.
      lwwMap.merge('user1', {
        value,
        timestamp: { millis: 9_000_000, counter: 0, nodeId: 'client' },
      });
      mockStorage.put.mockClear();

      const serverStamp = { millis: 1_000_000, counter: 0, nodeId: 'server' };
      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'SERVER_EVENT',
        payload: {
          mapName: 'users',
          eventType: 'PUT',
          key: 'user1',
          record: { value, timestamp: serverStamp },
        },
      });
      await jest.runAllTimersAsync();

      // Memory adopted the server's authoritative timestamp (converged)...
      expect(lwwMap.getRecord('user1')?.timestamp).toEqual(serverStamp);
      // ...and disk was written with the same server record (no skew).
      expect(mockStorage.put).toHaveBeenCalledWith(
        'users:user1',
        expect.objectContaining({ timestamp: serverStamp }),
      );
    });

    test('does NOT persist a stale echo superseded by a newer local write (F8 no data loss)', async () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      syncEngine.registerMap('users', lwwMap);
      await jest.runAllTimersAsync();

      // A newer local write the server hasn't seen yet.
      const newer = { name: 'Bob' };
      lwwMap.merge('user1', {
        value: newer,
        timestamp: { millis: 9_000_000, counter: 0, nodeId: 'client' },
      });
      mockStorage.put.mockClear();

      // A stale echo of an OLDER write (different value, lower ts) arrives.
      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'SERVER_EVENT',
        payload: {
          mapName: 'users',
          eventType: 'PUT',
          key: 'user1',
          record: {
            value: { name: 'Alice' },
            timestamp: { millis: 1_000_000, counter: 0, nodeId: 'server' },
          },
        },
      });
      await jest.runAllTimersAsync();

      // The newer local write is preserved, and the stale echo was NOT persisted.
      expect(lwwMap.get('user1')).toEqual(newer);
      expect(mockStorage.put).not.toHaveBeenCalled();
    });

    test('should handle SERVER_EVENT for ORMap add', async () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const orMap = new ORMap<string, any>(hlc);
      syncEngine.registerMap('tags', orMap);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      const timestamp = { millis: Date.now(), counter: 1, nodeId: 'remote-node' };

      ws.simulateMessage({
        type: 'SERVER_EVENT',
        payload: {
          mapName: 'tags',
          eventType: 'OR_ADD',
          key: 'item1',
          orRecord: { value: 'important', timestamp, tag: 'tag-123' },
        },
      });
      await jest.runAllTimersAsync();

      expect(orMap.get('item1')).toContain('important');
    });
  });

  describe('Garbage collection', () => {
    test('should handle GC_PRUNE message for LWWMap', async () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      syncEngine.registerMap('users', lwwMap);

      // Add and remove item to create tombstone
      lwwMap.set('user1', { name: 'Alice' });
      lwwMap.remove('user1');

      mockStorage.getAllKeys.mockResolvedValue(['users:user1']);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      const olderThan = { millis: Date.now() + 10000, counter: 0, nodeId: 'gc' };

      ws.simulateMessage({
        type: 'GC_PRUNE',
        payload: { olderThan },
      });
      await jest.runAllTimersAsync();

      // Tombstone should be pruned
    });

    test('should handle SYNC_RESET_REQUIRED', async () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      lwwMap.set('user1', { name: 'Alice' });
      syncEngine.registerMap('users', lwwMap);

      mockStorage.getAllKeys.mockResolvedValue(['users:user1']);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = [];

      ws.simulateMessage({
        type: 'SYNC_RESET_REQUIRED',
        payload: { mapName: 'users' },
      });
      await jest.runAllTimersAsync();

      // Should clear storage
      expect(mockStorage.remove).toHaveBeenCalledWith('users:user1');

      // Should send SYNC_INIT with timestamp 0
      const syncInit = ws.sentMessages.find((m) => m.type === 'SYNC_INIT');
      expect(syncInit).toBeDefined();
      expect(syncInit?.lastSyncTimestamp).toBe(0);
    });
  });

  describe('Local queries', () => {
    test('should run local query with where filter', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:user1', 'users:user2', 'posts:post1']);
      mockStorage.get
        .mockResolvedValueOnce({ value: { name: 'Alice', active: true }, timestamp: {} })
        .mockResolvedValueOnce({ value: { name: 'Bob', active: false }, timestamp: {} });

      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const results = await syncEngine.runLocalQuery('users', { where: { active: true } });

      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Alice');
    });

    test('should run local query with predicate filter', async () => {
      mockStorage.getAllKeys.mockResolvedValue(['users:user1', 'users:user2']);
      mockStorage.get
        .mockResolvedValueOnce({ value: { name: 'Alice', age: 25 }, timestamp: {} })
        .mockResolvedValueOnce({ value: { name: 'Bob', age: 35 }, timestamp: {} });

      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const results = await syncEngine.runLocalQuery('users', {
        predicate: { op: 'gt', attribute: 'age', value: 30 },
      });

      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Bob');
    });
  });

  describe('Timestamp synchronization', () => {
    test('should update HLC on incoming message with timestamp', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      const remoteTimestamp = { millis: Date.now() + 5000, counter: 10, nodeId: 'server' };

      ws.simulateMessage({
        type: 'AUTH_ACK',
        timestamp: remoteTimestamp,
      });
      await jest.runAllTimersAsync();

      expect(mockStorage.setMeta).toHaveBeenCalledWith('lastSyncTimestamp', remoteTimestamp.millis);
    });
  });

  describe('Error handling', () => {
    test('should handle malformed JSON message gracefully', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;

      // Simulate malformed JSON
      if (ws.onmessage) {
        ws.onmessage({ data: 'invalid json {{{' });
      }
      await jest.runAllTimersAsync();

      // Should not crash, engine should still work
      expect(MockWebSocket.instances).toHaveLength(1);
    });

    test('should handle WebSocket error event', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;

      if (ws.onerror) {
        ws.onerror(new Error('Connection failed'));
      }
      await jest.runAllTimersAsync();

      // Should not crash
      expect(MockWebSocket.instances).toHaveLength(1);
    });
  });

  describe('HLC access', () => {
    test('should provide access to HLC instance', () => {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();

      expect(hlc).toBeInstanceOf(HLC);
      expect(hlc.now()).toBeDefined();
    });
  });

  describe('topic offline queue', () => {
    it('queues topic messages when offline', async () => {
      syncEngine = new SyncEngine(config);
      // Advance only enough to flush onopen (0ms timer), not the 500ms grace timer.
      // Without a token, the engine waits for AUTH_REQUIRED during the grace window —
      // messages published during this window are queued (not yet authenticated).
      jest.advanceTimersByTime(0);
      await Promise.resolve();

      syncEngine.publishTopic('chat', { message: 'hello' });
      syncEngine.publishTopic('chat', { message: 'world' });

      const status = syncEngine.getTopicQueueStatus();
      expect(status.size).toBe(2);
      expect(status.maxSize).toBe(100); // default
    });

    it('respects maxSize with drop-oldest strategy', async () => {
      syncEngine = new SyncEngine({
        ...config,
        topicQueue: { maxSize: 3, strategy: 'drop-oldest' },
      });
      // Advance only enough to flush onopen, not the 500ms grace timer.
      jest.advanceTimersByTime(0);
      await Promise.resolve();

      // Queue 5 messages with maxSize 3
      for (let i = 0; i < 5; i++) {
        syncEngine.publishTopic('chat', { index: i });
      }

      const status = syncEngine.getTopicQueueStatus();
      expect(status.size).toBe(3);
      expect(status.maxSize).toBe(3);
    });

    it('respects drop-newest strategy', async () => {
      syncEngine = new SyncEngine({
        ...config,
        topicQueue: { maxSize: 2, strategy: 'drop-newest' },
      });
      // Advance only enough to flush onopen, not the 500ms grace timer.
      jest.advanceTimersByTime(0);
      await Promise.resolve();

      syncEngine.publishTopic('chat', { index: 0 });
      syncEngine.publishTopic('chat', { index: 1 });
      syncEngine.publishTopic('chat', { index: 2 }); // dropped

      const status = syncEngine.getTopicQueueStatus();
      expect(status.size).toBe(2);
    });

    it('returns correct default config', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const status = syncEngine.getTopicQueueStatus();
      expect(status.maxSize).toBe(100);
    });

    it('flushes queued messages on AUTH_ACK', async () => {
      // Use a token-configured engine so it enters AUTHENTICATING immediately
      // (waiting for AUTH_ACK) — messages published in this window are queued.
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');

      // Flush onopen without running the full timer chain.
      jest.advanceTimersByTime(0);
      await Promise.resolve();

      const ws = MockWebSocket.getLastInstance()!;
      // Engine is in AUTHENTICATING (AUTH was sent, waiting for AUTH_ACK).
      ws.sentMessages = [];

      // Publish while in AUTHENTICATING — not yet fully authenticated.
      syncEngine.publishTopic('chat', { message: 'queued1' });
      syncEngine.publishTopic('chat', { message: 'queued2' });

      expect(syncEngine.getTopicQueueStatus().size).toBe(2);

      // Simulate AUTH_ACK → drives to CONNECTED and flushes queue.
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // Queue should be flushed
      expect(syncEngine.getTopicQueueStatus().size).toBe(0);

      // Messages should have been sent
      const topicPubs = ws.sentMessages.filter((m) => m.type === 'TOPIC_PUB');
      expect(topicPubs).toHaveLength(2);
      expect(topicPubs[0].payload.data).toEqual({ message: 'queued1' });
      expect(topicPubs[1].payload.data).toEqual({ message: 'queued2' });
    });
  });

  describe('Batch delegation (sendBatch)', () => {
    // Mock IConnectionProvider with optional sendBatch (simulates cluster mode)
    function createMockClusterProvider() {
      const handlers = new Map<string, Set<(...args: any[]) => void>>();

      const mockConnection = {
        send: jest.fn(),
        close: jest.fn(),
        readyState: 1,
      };

      const provider = {
        connect: jest.fn().mockImplementation(async () => {
          // Trigger connected event after microtask
          setTimeout(() => {
            const set = handlers.get('connected');
            if (set) set.forEach((h) => h('mock-node'));
          }, 0);
        }),
        getConnection: jest.fn().mockReturnValue(mockConnection),
        getAnyConnection: jest.fn().mockReturnValue(mockConnection),
        isConnected: jest.fn().mockReturnValue(true),
        getConnectedNodes: jest.fn().mockReturnValue(['mock-node']),
        on: jest.fn().mockImplementation((event: string, handler: (...args: any[]) => void) => {
          if (!handlers.has(event)) handlers.set(event, new Set());
          handlers.get(event)!.add(handler);
        }),
        off: jest.fn(),
        send: jest.fn(),
        sendBatch: jest.fn().mockImplementation((ops: Array<{ key: string; message: any }>) => {
          const results = new Map<string, boolean>();
          for (const op of ops) results.set(op.key, true);
          return results;
        }),
        forceReconnect: jest.fn(),
        close: jest.fn().mockResolvedValue(undefined),
        _handlers: handlers,
      };

      return provider;
    }

    function simulateProviderMessage(
      provider: ReturnType<typeof createMockClusterProvider>,
      message: any,
    ) {
      const set = provider._handlers.get('message');
      if (set) {
        const data = serialize(message);
        const buf = new Uint8Array(data).buffer;
        set.forEach((h) => h('mock-node', buf));
      }
    }

    test('should delegate to sendBatch when provider implements it', async () => {
      const clusterProvider = createMockClusterProvider();

      const pendingOps: OpLogEntry[] = [
        {
          id: '1',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
        {
          id: '2',
          mapName: 'users',
          opType: 'PUT',
          key: 'user2',
          synced: false,
          timestamp: { millis: 1001, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      syncEngine = new SyncEngine({
        ...config,
        connectionProvider: clusterProvider as any,
      });
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      // Simulate AUTH_ACK to trigger syncPendingOperations
      simulateProviderMessage(clusterProvider, { type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // sendBatch should have been called with pending ops
      expect(clusterProvider.sendBatch).toHaveBeenCalledTimes(1);
      const callArgs = clusterProvider.sendBatch.mock.calls[0][0];
      expect(callArgs).toHaveLength(2);
      expect(callArgs[0].key).toBe('user1');
      expect(callArgs[1].key).toBe('user2');
      // Each message is the full OpLogEntry
      expect(callArgs[0].message.mapName).toBe('users');
      expect(callArgs[1].message.mapName).toBe('users');
    });

    test('should fall back to single OP_BATCH when provider lacks sendBatch', async () => {
      // SingleServerProvider does not implement sendBatch
      const pendingOps = [
        {
          id: '1',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      syncEngine = new SyncEngine(config); // uses SingleServerProvider (no sendBatch)
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = [];

      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      const opBatch = ws.sentMessages.find((m: any) => m.type === 'OP_BATCH');
      expect(opBatch).toBeDefined();
      expect(opBatch?.payload?.ops).toHaveLength(1);
    });

    test('should log warning when sendBatch reports failed keys', async () => {
      const clusterProvider = createMockClusterProvider();

      // Override sendBatch to report a failure for user2
      clusterProvider.sendBatch.mockImplementation((ops: Array<{ key: string; message: any }>) => {
        const results = new Map<string, boolean>();
        for (const op of ops) results.set(op.key, op.key !== 'user2');
        return results;
      });

      const pendingOps: OpLogEntry[] = [
        {
          id: '1',
          mapName: 'users',
          opType: 'PUT',
          key: 'user1',
          synced: false,
          timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
        },
        {
          id: '2',
          mapName: 'users',
          opType: 'PUT',
          key: 'user2',
          synced: false,
          timestamp: { millis: 1001, counter: 0, nodeId: 'test' },
        },
      ];
      mockStorage.getPendingOps.mockResolvedValue(pendingOps as any);

      // require() accesses the Jest-mocked logger module to spy on the warn method
      // eslint-disable-next-line @typescript-eslint/no-require-imports
      const loggerModule = require('../utils/logger');
      const warnSpy = jest.spyOn(loggerModule.logger, 'warn');

      syncEngine = new SyncEngine({
        ...config,
        connectionProvider: clusterProvider as any,
      });
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      simulateProviderMessage(clusterProvider, { type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      expect(clusterProvider.sendBatch).toHaveBeenCalledTimes(1);

      // Warning should include the failed key
      expect(warnSpy).toHaveBeenCalledWith(
        expect.objectContaining({ failedKeys: ['user2'], count: 1 }),
        'Some batch operations failed to send',
      );

      warnSpy.mockRestore();
    });
  });

  describe('HLC timestamp guard', () => {
    test('PONG message with raw numeric timestamp must not poison the HLC', async () => {
      syncEngine = new SyncEngine(config);
      syncEngine.setAuthToken('test-token');
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance()!;

      // Authenticate
      ws.simulateMessage({ type: 'AUTH_ACK' });
      await jest.runAllTimersAsync();

      // Simulate PONG — has a flat numeric `timestamp` field, not an HLC Timestamp struct.
      // Before the fix, this poisoned the HLC with NaN via Number(undefined).
      ws.simulateMessage({ type: 'PONG', timestamp: 1709712000000, serverTime: 1709712000001 });
      await jest.runAllTimersAsync();

      // Record an operation — its timestamp comes from the HLC.
      // If HLC were poisoned, millis would be NaN.
      const timestamp = { millis: Date.now(), counter: 0, nodeId: 'test-node' };
      const record = { value: 'hello', timestamp };
      await syncEngine.recordOperation('test-map', 'PUT', 'key1', { record, timestamp });

      // The OP_BATCH sent to the server should contain a valid timestamp
      const opBatch = ws.sentMessages.find((m) => m.type === 'OP_BATCH');
      expect(opBatch).toBeDefined();
      const sentTimestamp = opBatch.payload.ops[0].timestamp;
      expect(Number.isFinite(sentTimestamp.millis)).toBe(true);
      expect(sentTimestamp.millis).toBeGreaterThan(0);
    });
  });

  describe('Confirmed-apply ACK (apply-not-receive)', () => {
    // Uses an OR-Map + a driven held-set snapshot: since the cross-map
    // min-barrier landed, the confirmed-apply ACK is only reachable THROUGH
    // the barrier (epochs are OR-tombstone machinery; an engine holding no
    // OR-Maps deliberately never ACKs). With a single held OR-Map the
    // observable apply-not-receive behavior is unchanged from the original
    // contract these tests pin.
    async function startEngineWithMap() {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const orMap = new ORMap<string, any>(hlc);
      syncEngine.registerMap('users', orMap);
      await jest.runAllTimersAsync();
      const ws = MockWebSocket.getLastInstance()!;
      // Drive the held-set snapshot + sync-init kickoff deterministically
      // (same rationale as the min-barrier block's helper below).
      await (syncEngine as any).startMerkleSync();
      ws.sentMessages = []; // clear the auth/hello handshake + sync-init frames
      return ws;
    }

    let eventSeq = 0;
    function batchEvent(key: string, epoch?: number) {
      eventSeq += 1;
      return {
        mapName: 'users',
        eventType: 'OR_ADD' as const,
        key,
        orRecord: {
          value: { k: key },
          tag: `${Date.now()}:${eventSeq}:remote`,
          timestamp: { millis: Date.now(), counter: eventSeq, nodeId: 'remote' },
        },
        ...(epoch !== undefined ? { epoch } : {}),
      };
    }

    test('emits CLIENT_APPLY_ACK with the highest epoch ONLY after durable commit', async () => {
      const ws = await startEngineWithMap();

      // Gate the durable put so we can observe ordering: the ACK must NOT be sent
      // until the IndexedDB commit resolves (apply-not-receive).
      let resolvePut!: () => void;
      mockStorage.put.mockImplementationOnce(
        () => new Promise<void>((r) => (resolvePut = () => r())),
      );

      const applied = (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a', 5), batchEvent('b', 3)] },
      }) as Promise<void>;

      // Let the marker-first `setMeta` (eager `:ormap` existence marker, written
      // before the gated `put` on the first server-origin persist for this map)
      // settle so the gated `put` is actually reached before we assert on it.
      await jest.advanceTimersByTimeAsync(0);

      // Before the durable commit resolves, no ACK has been sent (not on receive).
      expect(ws.sentMessages.find((m) => m.type === 'CLIENT_APPLY_ACK')).toBeUndefined();

      resolvePut();
      await applied;

      const ack = ws.sentMessages.find((m) => m.type === 'CLIENT_APPLY_ACK');
      expect(ack).toBeDefined();
      expect(ack.cursor).toBe(5); // the highest epoch in the batch, inclusive
    });

    test('cursor is cumulative-monotonic: a non-advancing epoch is not re-sent', async () => {
      const ws = await startEngineWithMap();

      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a', 5)] },
      });
      // A lower/equal epoch later must NOT send another ACK.
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('b', 4)] },
      });
      let acks = ws.sentMessages.filter((m) => m.type === 'CLIENT_APPLY_ACK');
      expect(acks.map((m) => m.cursor)).toEqual([5]);

      // A higher epoch advances the cursor and sends a fresh ACK.
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('c', 7)] },
      });
      acks = ws.sentMessages.filter((m) => m.type === 'CLIENT_APPLY_ACK');
      expect(acks.map((m) => m.cursor)).toEqual([5, 7]);
    });

    test('epoch-less events (current server wire) emit no ACK (inert, not incorrect)', async () => {
      const ws = await startEngineWithMap();
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a'), batchEvent('b')] },
      });
      expect(ws.sentMessages.find((m) => m.type === 'CLIENT_APPLY_ACK')).toBeUndefined();
    });

    test('a failed ACK send does not advance the cursor; a later apply retries it', async () => {
      await startEngineWithMap();
      const sendSpy = jest.spyOn(syncEngine as any, 'sendMessage');

      // The socket cannot take the ACK (disconnected / buffer full): sendMessage
      // returns false for the ACK frame. The cursor must NOT advance — else the ACK
      // for this epoch is dropped forever (monotone: a lower epoch is never re-sent).
      sendSpy.mockImplementation((m: any) => m?.type !== 'CLIENT_APPLY_ACK');
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a', 5)] },
      });
      expect((syncEngine as any).lastAckedEpoch).toBe(0);

      // Socket recovers; the next apply of the same epoch re-attempts the ACK.
      const acks: number[] = [];
      sendSpy.mockImplementation((m: any) => {
        if (m?.type === 'CLIENT_APPLY_ACK') acks.push(m.cursor);
        return true;
      });
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a', 5)] },
      });
      expect(acks).toEqual([5]);
      expect((syncEngine as any).lastAckedEpoch).toBe(5);
    });

    test('setAuthToken resets the confirmed-apply cursor (new principal → independent cursor)', async () => {
      await startEngineWithMap();
      await (syncEngine as any).handleServerBatchEvent({
        payload: { events: [batchEvent('a', 5)] },
      });
      expect((syncEngine as any).lastAckedEpoch).toBe(5);

      // A new token may name a different principal, whose server-side cursor is
      // independent — the local high-water-mark must reset so its ACKs are not
      // suppressed by the prior identity's epoch.
      (syncEngine as any).setAuthToken('new-token-for-a-different-principal');
      expect((syncEngine as any).lastAckedEpoch).toBe(0);
    });
  });

  // Review v1 Major fix: the covering-epoch ACK is a device-wide cursor, but was
  // previously confirmed off a SINGLE OR-Map's sync completion — a client could
  // ACK past a tombstone stamped for an OTHER held map it had never received.
  // These tests exercise the per-map min-barrier (SyncEngine.applyMapCoverage)
  // that gates every covering-epoch ACK on ALL held OR-Maps having proven
  // delivery, not just the map that happened to sync first.
  describe('Cross-map covering-epoch ACK min-barrier', () => {
    async function startEngineWithOrMaps(mapNames: string[]) {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      for (const name of mapNames) {
        syncEngine.registerMap(name, new ORMap<string, any>(hlc));
      }
      // Let the mock WebSocket finish its async open (see MockWebSocket's
      // constructor setTimeout) so sendMessage can actually transmit below.
      await jest.runAllTimersAsync();
      const ws = MockWebSocket.getLastInstance()!;
      // Directly drive the (now async) held-set snapshot + sync-init kickoff —
      // the exact call handleAuthAck makes fire-and-forget — rather than
      // relying on the ~500ms auth-optional grace-timer race under fake
      // timers, so the snapshot is deterministically resolved before the test
      // body runs.
      await (syncEngine as any).startMerkleSync();
      ws.sentMessages = []; // clear the auth/hello handshake + sync-init frames
      return ws;
    }

    function acksOn(ws: any): number[] {
      return ws.sentMessages
        .filter((m: any) => m.type === 'CLIENT_APPLY_ACK')
        .map((m: any) => m.cursor);
    }

    test('(a) ACK never exceeds the MIN coverage across held maps (multi-map interleaving)', async () => {
      const ws = await startEngineWithOrMaps(['tags', 'other_map']);

      // 'tags' completes its sync round-trip first and conveys covering epoch 5.
      (syncEngine as any).applyMapCoverage('tags', 5);
      // The barrier stalls: 'other_map' has not synced on this connection yet,
      // so its coverage is still 0 — the MIN across the held set is 0.
      expect(acksOn(ws)).toEqual([]);

      // 'other_map' now completes at the same covering epoch.
      (syncEngine as any).applyMapCoverage('other_map', 5);
      expect(acksOn(ws)).toEqual([5]);
    });

    test('(a) the barrier advances incrementally to the current MIN, never past a lagging map', async () => {
      const ws = await startEngineWithOrMaps(['tags', 'other_map']);

      (syncEngine as any).applyMapCoverage('tags', 10);
      expect(acksOn(ws)).toEqual([]); // other_map still 0

      (syncEngine as any).applyMapCoverage('other_map', 3);
      expect(acksOn(ws)).toEqual([3]); // MIN(10, 3)

      (syncEngine as any).applyMapCoverage('tags', 20);
      expect(acksOn(ws)).toEqual([3]); // still MIN(20, 3) = 3, no new ACK

      (syncEngine as any).applyMapCoverage('other_map', 7);
      expect(acksOn(ws)).toEqual([3, 7]); // MIN(20, 7) = 7 advances
    });

    test('(b) a persisted-but-not-instantiated ORMap store is enumerated into the held-set and blocks the ACK until it syncs', async () => {
      mockStorage.getAllMetaKeys.mockResolvedValue(['__sys__:archive:tombstones']);
      mockStorage.getMeta.mockResolvedValue([]);
      mockStorage.getAllKeys.mockResolvedValue([]);

      const ws = await startEngineWithOrMaps(['tags']);

      // 'archive' was never registered via registerMap this session — it is only
      // discoverable through the storage adapter's persisted meta keys.
      (syncEngine as any).applyMapCoverage('tags', 5);
      expect(acksOn(ws)).toEqual([]); // 'archive' held but unsynced this connection

      (syncEngine as any).applyMapCoverage('archive', 5);
      expect(acksOn(ws)).toEqual([5]);
    });

    test('(c) an empty held-set emits no ACK at all', async () => {
      const ws = await startEngineWithOrMaps([]);
      (syncEngine as any).applyMapCoverage('stray-map', 5);
      expect(acksOn(ws)).toEqual([]);
    });

    test('(d) a map opened AFTER the connection snapshot joins with coverage 0 and blocks further advance', async () => {
      const ws = await startEngineWithOrMaps(['tags']);

      (syncEngine as any).applyMapCoverage('tags', 5);
      expect(acksOn(ws)).toEqual([5]);

      // A second OR-Map is opened mid-connection (e.g. client.getORMap('late')),
      // AFTER the held-set snapshot was already taken.
      const hlc = syncEngine!.getHLC();
      syncEngine!.registerMap('late', new ORMap<string, any>(hlc));

      // Further advances on the already-covered map are blocked by 'late's
      // fresh 0 coverage — the barrier never retroactively narrows OR widens,
      // it just now includes 'late' at 0.
      (syncEngine as any).applyMapCoverage('tags', 9);
      expect(acksOn(ws)).toEqual([5]); // no new ACK

      // Once 'late' reports its own coverage, the MIN can advance again.
      (syncEngine as any).applyMapCoverage('late', 9);
      expect(acksOn(ws)).toEqual([5, 9]);
    });

    test('held-set snapshot + coverage reset once per connection: a reconnect re-derives both fresh', async () => {
      await startEngineWithOrMaps(['tags', 'other_map']);
      (syncEngine as any).applyMapCoverage('tags', 5);
      (syncEngine as any).applyMapCoverage('other_map', 5);
      expect((syncEngine as any).orMapCoverage.get('tags')).toBe(5);
      expect((syncEngine as any).orMapCoverage.get('other_map')).toBe(5);

      // Simulate a fresh connection (new held-map snapshot + coverage reset).
      await (syncEngine as any).startMerkleSync();

      // Both maps' coverage resets to 0 on the new connection — a stale
      // cross-connection coverage value must never silently license an ACK on
      // the new connection without a fresh sync round-trip proving delivery.
      expect((syncEngine as any).orMapCoverage.get('tags')).toBe(0);
      expect((syncEngine as any).orMapCoverage.get('other_map')).toBe(0);
      expect((syncEngine as any).heldOrMapNames.has('tags')).toBe(true);
      expect((syncEngine as any).heldOrMapNames.has('other_map')).toBe(true);
    });

    test('held-set enumeration failure fail-closes ALL ACKs on the connection (never a partial barrier)', async () => {
      // A storage-adapter failure must never degrade to an in-memory-only
      // held-set: a persisted store the snapshot silently missed could hold
      // un-received tombstones while the device's ACK advances past them.
      mockStorage.getAllMetaKeys.mockRejectedValue(new Error('idb unavailable'));

      const ws = await startEngineWithOrMaps(['tags', 'other_map']);
      expect((syncEngine as any).heldSetIncomplete).toBe(true);

      // Even after EVERY in-memory map proves coverage, no ACK may be emitted —
      // the universe the MIN ranges over is unknown for this connection.
      (syncEngine as any).applyMapCoverage('tags', 5);
      (syncEngine as any).applyMapCoverage('other_map', 5);
      expect(acksOn(ws)).toEqual([]);

      // A later connection whose enumeration succeeds recovers normally.
      mockStorage.getAllMetaKeys.mockResolvedValue([]);
      await (syncEngine as any).startMerkleSync();
      ws.sentMessages = [];
      expect((syncEngine as any).heldSetIncomplete).toBe(false);
      (syncEngine as any).applyMapCoverage('tags', 6);
      (syncEngine as any).applyMapCoverage('other_map', 6);
      expect(acksOn(ws)).toEqual([6]);
    });

    test('server push events route their epoch through the min-barrier, never around it', async () => {
      const ws = await startEngineWithOrMaps(['tags', 'other_map']);

      // A live SERVER_EVENT for 'tags' carrying an epoch proves delivery for
      // 'tags' ONLY — with 'other_map' still uncovered, no device-wide ACK.
      await (syncEngine as any).handleServerEvent({
        type: 'SERVER_EVENT',
        payload: { mapName: 'tags', eventType: 'OR_ADD', key: 'k', epoch: 7 },
      });
      expect(acksOn(ws)).toEqual([]);

      // A batch event covering 'other_map' at the same epoch completes the MIN;
      // the ACK is exactly MIN(7, 7) — the batch's own epochs also went through
      // the barrier per-event (no batch-wide highest-epoch shortcut).
      await (syncEngine as any).handleServerBatchEvent({
        type: 'SERVER_BATCH_EVENT',
        payload: {
          events: [{ mapName: 'other_map', eventType: 'OR_ADD', key: 'k2', epoch: 7 }],
        },
      });
      expect(acksOn(ws)).toEqual([7]);
    });
  });

  describe('OR-Map existence marker + legacy backfill (add-only enumeration gap)', () => {
    const rec = (n: number) => ({
      value: { n },
      tag: `1:${n}:node`,
      timestamp: { millis: n, counter: 0, nodeId: 'node' },
    });

    // A storage adapter whose kv + meta stores are real Maps, so a marker written
    // by backfill / the persist helpers is actually observable by a later
    // enumeration — the integration the plain jest.fn mock cannot exercise.
    function statefulStorage(seed?: {
      kv?: Record<string, unknown>;
      meta?: Record<string, unknown>;
    }): jest.Mocked<IStorageAdapter> {
      const kv = new Map<string, unknown>(Object.entries(seed?.kv ?? {}));
      const meta = new Map<string, unknown>(Object.entries(seed?.meta ?? {}));
      const s = createMockStorageAdapter();
      s.get.mockImplementation(async (k: string) => kv.get(k));
      s.put.mockImplementation(async (k: string, v: unknown) => {
        kv.set(k, v);
      });
      s.remove.mockImplementation(async (k: string) => {
        kv.delete(k);
      });
      s.getAllKeys.mockImplementation(async () => [...kv.keys()]);
      s.getMeta.mockImplementation(async (k: string) => meta.get(k));
      s.setMeta.mockImplementation(async (k: string, v: unknown) => {
        meta.set(k, v);
      });
      s.getAllMetaKeys.mockImplementation(async () => [...meta.keys()]);
      s.commitWrite.mockImplementation(async (mutations) => {
        for (const m of mutations) {
          const store = m.store === 'meta' ? meta : kv;
          if (m.type === 'remove') store.delete(m.key);
          else store.set(m.key, m.value);
        }
        return 1;
      });
      return s;
    }

    async function startEngine(storage: jest.Mocked<IStorageAdapter>, openMaps: string[] = []) {
      config.storageAdapter = storage;
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      for (const name of openMaps) {
        syncEngine.registerMap(name, new ORMap<string, unknown>(hlc));
      }
      await jest.runAllTimersAsync();
      const ws = MockWebSocket.getLastInstance()!;
      await (syncEngine as any).startMerkleSync();
      ws.sentMessages = [];
      return ws;
    }

    function acksOn(ws: any): number[] {
      return ws.sentMessages
        .filter((m: any) => m.type === 'CLIENT_APPLY_ACK')
        .map((m: any) => m.cursor);
    }

    test('THE REGRESSION: a legacy add-only persisted store (records, no :tombstones, no :ormap marker) is discovered by backfill and blocks the device ACK', async () => {
      // Pre-fix device: `shared` was written add-only in a prior session (kv records
      // only), never removed → no `:tombstones` key, and predates the eager marker →
      // no `:ormap` marker. Backfill has not run (done-flag unset). This is the exact
      // store the old snapshot missed, letting the device ACK past its un-received
      // tombstones (the reopened cross-map resurrection vector).
      const storage = statefulStorage({ kv: { 'shared:k1': [rec(1)] } });

      const ws = await startEngine(storage, ['tags']);

      // Backfill stamped the marker and it is now enumerated into the held-set.
      expect(await storage.getMeta('__sys__:shared:ormap')).toBe(1);
      expect((syncEngine as any).heldOrMapNames.has('shared')).toBe(true);

      // 'tags' conveying the server-wide covering epoch no longer licenses a device
      // ACK: 'shared' is held and unsynced this connection, so the MIN stalls at 0.
      (syncEngine as any).applyMapCoverage('tags', 5);
      expect(acksOn(ws)).toEqual([]);

      // Only once 'shared' proves its own delivery does the ACK advance.
      (syncEngine as any).applyMapCoverage('shared', 5);
      expect(acksOn(ws)).toEqual([5]);
    });

    test('local OR write stamps the eager :ormap marker transactionally (in the commitWrite), once per session', async () => {
      const storage = statefulStorage();
      config.storageAdapter = storage;
      syncEngine = new SyncEngine(config);
      syncEngine.registerMap('notes', new ORMap<string, unknown>(syncEngine.getHLC()));

      const r = rec(1);
      const mutations = [
        { store: 'kv' as const, type: 'put' as const, key: 'notes:k', value: [r] },
      ];
      await syncEngine.recordOperation(
        'notes',
        'OR_ADD',
        'k',
        { orRecord: r as any, timestamp: r.timestamp as any },
        mutations,
      );

      // Marker landed in the SAME commit as the data (atomic — no crash window).
      const firstCommit = storage.commitWrite.mock.calls[0][0];
      expect(firstCommit).toContainEqual({
        store: 'meta',
        type: 'put',
        key: '__sys__:notes:ormap',
        value: 1,
      });
      expect(await storage.getMeta('__sys__:notes:ormap')).toBe(1);

      // Second write to the same map does NOT re-stamp the marker (session guard).
      await syncEngine.recordOperation(
        'notes',
        'OR_ADD',
        'k2',
        { orRecord: rec(2) as any, timestamp: rec(2).timestamp as any },
        [{ store: 'kv', type: 'put', key: 'notes:k2', value: [rec(2)] }],
      );
      const secondCommit = storage.commitWrite.mock.calls[1][0];
      expect(secondCommit.some((m) => m.key === '__sys__:notes:ormap')).toBe(false);
    });

    test('server-origin persist writes the :ormap marker BEFORE the data (crash-safe ordering)', async () => {
      const storage = statefulStorage();
      config.storageAdapter = storage;
      syncEngine = new SyncEngine(config);
      const orMap = new ORMap<string, unknown>(syncEngine.getHLC());
      orMap.apply('k', rec(1) as any);
      syncEngine.registerMap('recv', orMap);

      await syncEngine.persistORMapKey('recv', 'k');

      const markerCallIdx = storage.setMeta.mock.calls.findIndex(
        (c) => c[0] === '__sys__:recv:ormap',
      );
      expect(markerCallIdx).toBeGreaterThanOrEqual(0);
      const markerOrder = storage.setMeta.mock.invocationCallOrder[markerCallIdx];
      const putOrder = storage.put.mock.invocationCallOrder[0];
      // Marker-first: a crash between them leaves the map DISCOVERABLE, never a
      // durable record with no marker (which would reopen the invisible-store hole).
      expect(markerOrder).toBeLessThan(putOrder);
    });

    test('backfill discriminates OR (array) from LWW (object) and is gated by the durable done-flag', async () => {
      const storage = statefulStorage({
        kv: {
          'orm:1': [rec(1)],
          'lww:1': { value: 42, timestamp: { millis: 1, counter: 0, nodeId: 'n' } },
        },
      });

      await startEngine(storage);

      expect(await storage.getMeta('__sys__:orm:ormap')).toBe(1); // array → OR, marked
      expect(await storage.getMeta('__sys__:lww:ormap')).toBeUndefined(); // object → LWW, not marked
      expect(await storage.getMeta('__sys__:ormapBackfillDone')).toBe(true);

      // A second connection does NOT re-scan the keyspace (done-flag gates it): no
      // further get() calls are issued by backfill (the OR store is already in
      // this.maps from the first restore, so nothing re-reads it either).
      const getCallsBefore = storage.get.mock.calls.length;
      await (syncEngine as any).startMerkleSync();
      expect(storage.get.mock.calls.length).toBe(getCallsBefore);
    });

    test('backfill failure fail-closes the connection (ACKs suppressed, done-flag left unset for retry)', async () => {
      const storage = statefulStorage({ kv: { 'x:1': [rec(1)] } });
      // The one-time scan's keyspace read fails — enumeration must NOT silently
      // proceed over a partial universe. Persistent (not Once) so both the auth-
      // driven fire-and-forget startMerkleSync AND the explicit one below see it.
      storage.getAllKeys.mockRejectedValue(new Error('idb unavailable'));

      const ws = await startEngine(storage);

      expect((syncEngine as any).heldSetIncomplete).toBe(true);
      // Done-flag left unset → the migration retries on the next connection.
      expect(await storage.getMeta('__sys__:ormapBackfillDone')).toBeUndefined();
      // No ACK may be emitted — the held universe is unknown for this connection.
      (syncEngine as any).applyMapCoverage('x', 5);
      expect(acksOn(ws)).toEqual([]);

      // A later connection whose scan succeeds recovers and stamps the marker.
      storage.getAllKeys.mockImplementation(async () => ['x:1']);
      await (syncEngine as any).startMerkleSync();
      expect((syncEngine as any).heldSetIncomplete).toBe(false);
      expect(await storage.getMeta('__sys__:x:ormap')).toBe(1);
    });
  });

  describe('Permanently refused operations (per-op verdicts)', () => {
    // Narrow views on the engine internals these tests must observe: the op log
    // itself, and the backpressure state an all-refused flush has to release.
    type EnginePrivates = {
      opLog: OpLogEntry[];
      backpressureController: {
        checkLowWaterMark: () => void;
        checkHighWaterMark: () => void;
        highWaterMarkEmitted: boolean;
        backpressurePaused: boolean;
      };
      recordRejection: (opId: string, record: unknown) => void;
      recordSyncStateTracker: { onAcknowledge: (op: OpLogEntry) => void };
      // Optional: an engine that keeps no registry of the batches it sent has no
      // such field, and a test that needs one must fail on an assertion, not on a
      // property read.
      inFlightBatches?: Map<string, Set<string>>;
    };
    const engine = () => syncEngine as unknown as EnginePrivates;

    const pendingOp = (id: string, key: string) => ({
      id,
      mapName: 'users',
      opType: 'PUT',
      key,
      synced: false,
      timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
    });

    async function bootWith(
      ops: ReturnType<typeof pendingOp>[],
      overrides: Partial<typeof config> = {},
    ) {
      // Restored rows carry the engine-side shape (`opType`), which the durable
      // interface types more loosely than the engine writes it.
      mockStorage.getPendingOps.mockResolvedValue(ops as unknown as StoredOpLogEntry[]);
      syncEngine = new SyncEngine({ ...config, ...overrides });
      await jest.runAllTimersAsync();
      return MockWebSocket.getLastInstance()!;
    }

    const refusal = (opId: string, permanent = true) => ({
      type: 'OP_REJECTED',
      payload: { opId, reason: 'forbidden by policy', code: 4003, permanent },
    });

    const opLogOf = () => engine().opLog;

    // Everything a frame does when the engine reads it as a verdict on the op
    // log, checked together. The pending count alone would miss a frame that
    // left the count intact and still deleted durable rows, announced an
    // acknowledgement to the per-record tracker or released backpressure.
    async function expectNotAVerdict(inject: () => void) {
      const acknowledgeSpy = jest.spyOn(engine().recordSyncStateTracker, 'onAcknowledge');
      const lowWaterSpy = jest.spyOn(engine().backpressureController, 'checkLowWaterMark');
      const pendingBefore = syncEngine!.getPendingOpsCount();
      const opsBefore = opLogOf().map((o) => ({ id: o.id, synced: o.synced }));
      mockStorage.markOpsSynced.mockClear();

      inject();
      await jest.runAllTimersAsync();

      expect(syncEngine!.getPendingOpsCount()).toBe(pendingBefore);
      expect(mockStorage.markOpsSynced).not.toHaveBeenCalled();
      expect(opLogOf().map((o) => ({ id: o.id, synced: o.synced }))).toEqual(opsBefore);
      expect(acknowledgeSpy).not.toHaveBeenCalled();
      expect(lowWaterSpy).not.toHaveBeenCalled();
      acknowledgeSpy.mockRestore();
      lowWaterSpy.mockRestore();
    }

    test('waitForOpSynced answers rejected for an op already retired and spliced out', async () => {
      const ws = await bootWith([pendingOp('7', 'user7')]);

      ws.simulateMessage(refusal('7'));
      await jest.runAllTimersAsync();

      // Retired: durably deleted, gone from the op log, and no longer pending.
      expect(mockStorage.deleteOp).toHaveBeenCalledWith(7);
      expect(opLogOf().find((o) => o.id === '7')).toBeUndefined();
      expect(syncEngine!.getPendingOpsCount()).toBe(0);

      // The op is ABSENT, which is exactly the state the "acked and compacted"
      // fast path reads as 'synced'. The refusal registry is consulted first.
      await expect(syncEngine!.waitForOpSynced('7', 100)).resolves.toBe('rejected');
    });

    test('a later OP_ACK covering a refused op neither resurrects it nor reports it synced', async () => {
      const order: string[] = [];
      mockStorage.deleteOp.mockImplementation(
        () =>
          new Promise<void>((resolve) =>
            setTimeout(() => {
              order.push('deleteOp');
              resolve();
            }, 50),
          ),
      );
      mockStorage.markOpsSynced.mockImplementation(async () => {
        order.push('markOpsSynced');
      });

      const ws = await bootWith([pendingOp('3', 'user3'), pendingOp('5', 'user5')]);

      // Refusal then an ack whose numeric prefix would otherwise swallow the
      // refused op — the two arrive back to back, before the delete resolves.
      ws.simulateMessage(refusal('5'));
      ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '5', achievedLevel: 'APPLIED' } });
      await jest.runAllTimersAsync();

      await expect(syncEngine!.waitForOpSynced('5', 100)).resolves.toBe('rejected');
      // The prefix stops at the last op the server did NOT refuse.
      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(3);
      // Ordering is a mechanism, not a hope: markOpsSynced DELETES every durable
      // op at or below its id, so it must never run before the refused op's own
      // delete has resolved.
      expect(order).toEqual(['deleteOp', 'markOpsSynced']);
    });

    test('a refusal naming a non-numeric opId is kept, not thrown and not silently skipped', async () => {
      const errorSpy = jest.spyOn(logger, 'error');
      const ws = await bootWith([pendingOp('abc', 'userAbc')]);

      expect(() => ws.simulateMessage(refusal('abc'))).not.toThrow();
      await jest.runAllTimersAsync();

      // deleteOp addresses a durable row by numeric id, so this op cannot be
      // deleted at all: it stays in memory, visibly refused, rather than being
      // spliced away while it survives on disk.
      expect(mockStorage.deleteOp).not.toHaveBeenCalled();
      const kept = opLogOf().find((o) => o.id === 'abc');
      expect(kept?.rejected).toBe(true);
      expect(errorSpy).toHaveBeenCalled();
      await expect(syncEngine!.waitForOpSynced('abc', 100)).resolves.toBe('rejected');
      errorSpy.mockRestore();
    });

    test('a partial ack drives the full tail: durable mark, compaction, backpressure release', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ]);
      const controller = engine().backpressureController;
      const lowWaterSpy = jest.spyOn(controller, 'checkLowWaterMark');

      // Op 3 is refused earlier in the stream, then the ack names the other two.
      ws.simulateMessage(refusal('3'));
      await jest.runAllTimersAsync();
      mockStorage.markOpsSynced.mockClear();

      ws.simulateMessage({
        type: 'OP_ACK',
        payload: {
          // lastId would cover the refused op — the acceptance set, not the
          // prefix, decides what this ack covers.
          lastId: '3',
          results: [
            { opId: '1', success: true, achievedLevel: 'PERSISTED' },
            { opId: '2', success: true, achievedLevel: 'PERSISTED' },
          ],
        },
      });
      await jest.runAllTimersAsync();

      expect(mockStorage.markOpsSynced).toHaveBeenCalledTimes(1);
      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(2);
      expect(opLogOf()).toHaveLength(0);
      // Back to the pending count the client had before the batch was queued.
      expect(syncEngine!.getPendingOpsCount()).toBe(0);
      expect(lowWaterSpy).toHaveBeenCalled();
    });

    test('a present acceptance set is authoritative: the numeric prefix is not consulted', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('4', 'user4'),
      ]);

      // The set names 1 and 2; the prefix would additionally sweep in 4, which
      // the server said nothing about. When results is present it is the whole
      // coverage of the ack, so 4 stays pending.
      ws.simulateMessage({
        type: 'OP_ACK',
        payload: {
          lastId: '4',
          results: [
            { opId: '1', success: true, achievedLevel: 'PERSISTED' },
            { opId: '2', success: true, achievedLevel: 'PERSISTED' },
          ],
        },
      });
      await jest.runAllTimersAsync();

      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(2);
      expect(opLogOf().map((o) => o.id)).toEqual(['4']);
      expect(syncEngine!.getPendingOpsCount()).toBe(1);
    });

    test('a results entry reporting success:false is not an acceptance and is not durably deleted', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ]);

      // A foreign or future server may report a per-entry failure inside results
      // rather than as an OP_REJECTED frame. This server never does, but the
      // client is a protocol consumer: an entry it cannot read as an acceptance
      // must not be marked synced, and — because markOpsSynced is a durable
      // PREFIX delete — must not be swept away by a larger accepted id either.
      ws.simulateMessage({
        type: 'OP_ACK',
        payload: {
          lastId: '3',
          results: [
            { opId: '1', success: true, achievedLevel: 'PERSISTED' },
            { opId: '2', success: false, achievedLevel: 'PERSISTED', error: 'partition down' },
            { opId: '3', success: true, achievedLevel: 'PERSISTED' },
          ],
        },
      });
      await jest.runAllTimersAsync();

      // Op 2 has no verdict this exchange: still pending, never spliced, and no
      // refusal record — a failure is not a refusal.
      expect(opLogOf().map((o) => o.id)).toEqual(['2']);
      expect(opLogOf()[0].synced).toBe(false);
      expect(syncEngine!.getPendingOpsCount()).toBe(1);
      expect(syncEngine!.getRejectedOpCount()).toBe(0);

      // The durable prefix stops BELOW the failed id: 1 is deleted, 2 survives on
      // disk. Op 3 is accepted in memory but stays on disk until a later ack
      // moves the prefix past 2 — it re-sends after a restart, which is safe.
      expect(mockStorage.markOpsSynced).toHaveBeenCalledTimes(1);
      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(1);
    });

    test('a results entry that omits success is not an acceptance either', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ]);

      // Nothing validates an inbound frame at runtime, so `success` is a type-level
      // promise, not an enforced one. An entry carrying an error but no `success`
      // must land on the safe side of a durable PREFIX delete: an op left pending
      // is retried, an op deleted while unaccepted is gone.
      ws.simulateMessage({
        type: 'OP_ACK',
        payload: {
          lastId: '3',
          results: [
            { opId: '1', success: true, achievedLevel: 'PERSISTED' },
            { opId: '2', achievedLevel: 'PERSISTED', error: 'partition down' },
            { opId: '3', success: true, achievedLevel: 'PERSISTED' },
          ],
        },
      });
      await jest.runAllTimersAsync();

      expect(opLogOf().map((o) => o.id)).toEqual(['2']);
      expect(opLogOf()[0].synced).toBe(false);
      expect(mockStorage.markOpsSynced).toHaveBeenCalledTimes(1);
      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(1);
    });

    test('an empty acceptance set with a lastId that answers no sent batch is not a verdict', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ]);

      // An empty results array names no op, so it accepts nothing by itself and
      // the frame can only answer the batch whose last id it carries. The one
      // batch the engine sent ends in op 3; nothing was ever sent that ends in
      // op 2, so a numeric prefix read of this frame would retire ops 1 and 2 on
      // the word of a frame that answers no batch at all.
      await expectNotAVerdict(() =>
        ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '2', results: [] } }),
      );
    });

    test('an empty acceptance set accepts nothing even when its lastId is the last id of a sent batch', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ]);

      // The frame names the batch in flight and carries the level a real ack
      // carries, but its acceptance set is present and empty: the server listed
      // the ops it accepted and listed none. An op missing from `results` was not
      // accepted, so reading this frame as "no results, hence the whole batch"
      // would retire three writes on the word of a frame that accepts nothing.
      await expectNotAVerdict(() =>
        ws.simulateMessage({
          type: 'OP_ACK',
          payload: { lastId: '3', achievedLevel: 'APPLIED', results: [] },
        }),
      );

      // The batch is still on record, so its real acknowledgement still lands.
      expect([...(engine().inFlightBatches?.get('3') ?? [])]).toEqual(['1', '2', '3']);
      ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '3', achievedLevel: 'APPLIED' } });
      await jest.runAllTimersAsync();
      expect(syncEngine!.getPendingOpsCount()).toBe(0);
      expect(mockStorage.markOpsSynced.mock.calls).toEqual([[3]]);
    });

    test('an ack whose max accepted id is below the last applied one marks nothing again', async () => {
      const ws = await bootWith([
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('5', 'user5'),
      ]);

      ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '5', achievedLevel: 'APPLIED' } });
      await jest.runAllTimersAsync();
      expect(mockStorage.markOpsSynced).toHaveBeenCalledTimes(1);
      expect(mockStorage.markOpsSynced).toHaveBeenCalledWith(5);

      // Acks from two server dispatch tasks can interleave; a lower id would
      // delete a strictly smaller, already-deleted set.
      ws.simulateMessage({
        type: 'OP_ACK',
        payload: {
          lastId: '2',
          results: [{ opId: '2', success: true, achievedLevel: 'PERSISTED' }],
        },
      });
      await jest.runAllTimersAsync();
      expect(mockStorage.markOpsSynced).toHaveBeenCalledTimes(1);
    });

    test('a duplicate refusal produces no second refusal record and no second delete', async () => {
      const ws = await bootWith([pendingOp('4', 'user4')]);
      const recordSpy = jest.spyOn(engine(), 'recordRejection');

      ws.simulateMessage(refusal('4'));
      await jest.runAllTimersAsync();
      ws.simulateMessage(refusal('4'));
      await jest.runAllTimersAsync();

      // recordRejection is where a refusal becomes visible to a presentation
      // surface, so "called once" is "presented once".
      expect(recordSpy).toHaveBeenCalledTimes(1);
      expect(mockStorage.deleteOp).toHaveBeenCalledTimes(1);
      expect(syncEngine!.getRejectedOpCount()).toBe(1);
    });

    test('an all-refused flush releases the high water mark without any OP_ACK', async () => {
      const ws = await bootWith([pendingOp('1', 'user1'), pendingOp('2', 'user2')], {
        backpressure: { maxPendingOps: 2, highWaterMark: 0.5, lowWaterMark: 0.5 },
      });
      const controller = engine().backpressureController;
      const lowWaterSpy = jest.spyOn(controller, 'checkLowWaterMark');

      // The client sits at the high water mark with writes paused.
      controller.checkHighWaterMark();
      controller.backpressurePaused = true;
      expect(controller.highWaterMarkEmitted).toBe(true);
      let resumed = false;
      syncEngine!.onBackpressure('backpressure:low', () => {
        resumed = true;
      });

      // Every op of the flush is refused and the server sends NO ack at all, so
      // handleOpAck — the other backpressure release — never runs.
      ws.simulateMessage(refusal('1'));
      ws.simulateMessage(refusal('2'));
      await jest.runAllTimersAsync();

      expect(lowWaterSpy).toHaveBeenCalled();
      expect(syncEngine!.getPendingOpsCount()).toBe(0);
      expect(controller.highWaterMarkEmitted).toBe(false);
      expect(syncEngine!.isBackpressurePaused()).toBe(false);
      expect(resumed).toBe(true);
    });

    test('the refusal registry is bounded and evicts the oldest entries', async () => {
      const ws = await bootWith([]);

      for (let i = 1; i <= 1500; i++) {
        ws.simulateMessage(refusal(String(i)));
      }
      await jest.runAllTimersAsync();

      expect(syncEngine!.getRejectedOpCount()).toBe(1000);
      // Oldest 500 evicted, newest 1000 retained.
      expect(syncEngine!.getOpRejection('1')).toBeUndefined();
      expect(syncEngine!.getOpRejection('500')).toBeUndefined();
      expect(syncEngine!.getOpRejection('501')).toBeDefined();
      expect(syncEngine!.getOpRejection('1500')).toBeDefined();
    });

    test('a refusal that is not permanent leaves the op pending for a later flush', async () => {
      const ws = await bootWith([pendingOp('9', 'user9')]);

      ws.simulateMessage(refusal('9', false));
      await jest.runAllTimersAsync();

      expect(mockStorage.deleteOp).not.toHaveBeenCalled();
      expect(opLogOf().find((o) => o.id === '9')?.rejected).toBeUndefined();
      expect(syncEngine!.getRejectedOpCount()).toBe(0);
      expect(syncEngine!.getPendingOpsCount()).toBe(1);
    });

    describe('An OP_ACK is a verdict only on a sent batch', () => {
      // --- Frames ---

      const ack = (payload: Record<string, unknown>) => ({ type: 'OP_ACK', payload });
      // The results-less acknowledgement a supported server sends for a batch it
      // applied in full: the last id of the batch and the level it reached.
      const applied = (lastId: string) => ack({ lastId, achievedLevel: 'APPLIED' });

      // --- What the engine handed to the transport ---

      const batchIdsOf = (messages: any[]): string[][] =>
        messages
          .filter((m) => m?.type === 'OP_BATCH')
          .map((m) => m.payload.ops.map((o: { id: string }) => o.id));
      const batchesSentOn = (ws: MockWebSocket) => batchIdsOf(ws.sentMessages);
      const lastOf = <T>(items: T[]): T | undefined => items[items.length - 1];

      const inFlight = () => engine().inFlightBatches;

      const expectRetiredUpTo = (durablePrefix: number) => {
        expect(opLogOf()).toHaveLength(0);
        expect(syncEngine!.getPendingOpsCount()).toBe(0);
        expect(mockStorage.markOpsSynced.mock.calls).toEqual([[durablePrefix]]);
      };

      // --- Providers other than the default single-server WebSocket one ---
      //
      // Typed loosely on purpose. The engine decides what a results-less ack
      // means from two things a provider may or may not offer: a declared
      // `transport`, and a report of the batches `sendBatch` put on the wire.
      // A plain record lets one mock omit either, and lets a test assign
      // `transport` after the engine was built.

      function providerMock(extra: Record<string, unknown> = {}) {
        const handlers = new Map<string, Set<(...args: any[]) => void>>();
        const emit = (event: string, ...args: unknown[]) =>
          handlers.get(event)?.forEach((handler) => handler(...args));

        const provider: Record<string, any> = {
          connect: jest.fn().mockImplementation(async () => {
            setTimeout(() => emit('connected', 'mock-node'), 0);
          }),
          getConnection: jest.fn(),
          getAnyConnection: jest.fn(),
          isConnected: jest.fn().mockReturnValue(true),
          getConnectedNodes: jest.fn().mockReturnValue(['mock-node']),
          on: jest.fn().mockImplementation((event: string, handler: (...args: any[]) => void) => {
            if (!handlers.has(event)) handlers.set(event, new Set());
            handlers.get(event)!.add(handler);
          }),
          off: jest.fn(),
          send: jest.fn(),
          forceReconnect: jest.fn(),
          close: jest.fn().mockResolvedValue(undefined),
          ...extra,
        };

        return {
          provider,
          emit,
          /** Delivers a server frame to the engine, as the provider's `message` event. */
          deliver: (message: unknown) =>
            emit('message', 'mock-node', new Uint8Array(serialize(message)).buffer),
          /** Op ids of every OP_BATCH the engine sent through `send`, in send order. */
          sentBatches: () =>
            batchIdsOf(provider.send.mock.calls.map(([data]: [Uint8Array]) => deserialize(data))),
        };
      }

      /** HTTP provider: one OP_BATCH per flush through `send`; its acks carry no level. */
      const httpMock = () => providerMock({ transport: 'http' });

      /** Third-party provider that declares no transport at all. */
      const bareMock = () => providerMock();

      /**
       * Cluster provider. `sendBatch` splits a flush into one frame per target
       * node and reports each frame's op ids through its second argument, which
       * an engine that keeps no registry simply does not pass.
       *
       * - `nodeOf` routes an op id to a node; by default every op goes to one node.
       * - `script(...)` queues, per upcoming `sendBatch` call, the exact frames
       *   that call puts on the wire; an op in no scripted frame fails to send.
       * - `reportsBatches: false` models a provider that sends but never reports.
       */
      function clusterMock(
        options: { nodeOf?: (opId: string) => string; reportsBatches?: boolean } = {},
      ) {
        const { nodeOf = () => 'mock-node', reportsBatches = true } = options;
        const scripted: string[][][] = [];
        const frames: string[][] = [];

        const sendBatch = jest.fn(
          (
            operations: Array<{ key: string; message: { id: string } }>,
            onBatchSent?: (opIds: string[]) => void,
          ) => {
            let callFrames = scripted.shift();
            if (!callFrames) {
              const byNode = new Map<string, string[]>();
              for (const { message } of operations) {
                const node = nodeOf(message.id);
                if (!byNode.has(node)) byNode.set(node, []);
                byNode.get(node)!.push(message.id);
              }
              callFrames = [...byNode.values()];
            }

            const sent = new Set<string>();
            for (const frame of callFrames) {
              frames.push(frame);
              frame.forEach((id) => sent.add(id));
              if (reportsBatches) onBatchSent?.(frame);
            }
            return new Map(operations.map((op) => [op.key, sent.has(op.message.id)]));
          },
        );

        return {
          ...providerMock({ transport: 'websocket', sendBatch }),
          sendBatch,
          /** Every frame put on the wire so far, reported or not. */
          frames,
          script: (...calls: string[][][]) => {
            scripted.push(...calls);
          },
        };
      }

      const bootOn = (
        mock: { provider: Record<string, any> },
        ops: ReturnType<typeof pendingOp>[],
      ) =>
        bootWith(ops, {
          connectionProvider: mock.provider as unknown as SyncEngineConfig['connectionProvider'],
        });

      // --- Local writes with distinct durable ids ---

      // The default storage mock answers every append with id 1, which would put
      // every batch of a many-write test under one and the same last id.
      const issueOpIdsFrom = (first: number) => {
        let next = first;
        mockStorage.appendOpLog.mockImplementation(async () => next++);
      };

      const write = (key: string) => {
        const timestamp = { millis: 2000, counter: 0, nodeId: 'test-node' };
        return syncEngine!.recordOperation('users', 'PUT', key, {
          record: { value: { key }, timestamp },
          timestamp,
        });
      };

      // Three hundred unanswered writes must all stay pending, so no pending-op
      // ceiling may pause or drop any of them.
      const roomy = { backpressure: { maxPendingOps: 100_000 } };

      const range = (from: number, to: number) =>
        Array.from({ length: to - from + 1 }, (_, i) => from + i);

      const threeOps = () => [
        pendingOp('1', 'user1'),
        pendingOp('2', 'user2'),
        pendingOp('3', 'user3'),
      ];

      describe('old-server frame shapes (permanent guards of TG-SYNC-004)', () => {
        // These tests are the standing protection of the client rule against
        // servers that are ALREADY RELEASED and will keep running in the field
        // whatever a newer server does. Each one injects, byte for byte, a frame
        // such a server really sends. They must never be deleted, weakened or
        // skipped: once the current server stops producing these frames, nothing
        // else in the repository exercises them.

        test('old-server frame: a results-less OP_ACK whose lastId answers no sent batch is not a verdict', async () => {
          const ws = await bootWith(threeOps());
          expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

          // What a released server answers a push diff or an unsubscribe with: a
          // frame-counter value in lastId, no level, no results. It was never an
          // answer to an OP_BATCH.
          await expectNotAVerdict(() =>
            ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '900' } }),
          );
        });

        test('old-server frame: a non-numeric lastId with no acceptance set is not a verdict', async () => {
          const ws = await bootWith(threeOps());
          expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

          // What a released server answers an empty batch with. Read as "the
          // server took everything", it would retire every pending write.
          await expectNotAVerdict(() =>
            ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: 'unknown' } }),
          );
        });

        test('old-server frame: on HTTP, an ack without achievedLevel retires the answered batch', async () => {
          const http = httpMock();
          await bootOn(http, threeOps());
          expect(http.sentBatches()).toEqual([['1', '2', '3']]);

          // The HTTP acknowledgement of every release carries lastId and nothing
          // else. Demanding a level here would leave every HTTP write pending
          // forever.
          http.deliver({ type: 'OP_ACK', payload: { lastId: '3' } });
          await jest.runAllTimersAsync();

          expectRetiredUpTo(3);
        });

        test('old-server frame: on WebSocket, a results-less OP_ACK without achievedLevel is not a verdict even when its lastId is the last id of a sent batch', async () => {
          const ws = await bootWith(threeOps());
          expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

          // The same stray frame, at a counter value that happens to equal the
          // last id of the batch in flight. Matching a sent batch is not enough on
          // WebSocket: the acknowledgement of a real batch always carries the
          // level it reached, and this frame does not.
          await expectNotAVerdict(() =>
            ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '3' } }),
          );
        });
      });

      test('an ack for a batch sent before a connection loss and re-auth still retires exactly that batch', async () => {
        const ws = await bootWith(threeOps());
        expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

        ws.close();
        await jest.runAllTimersAsync();
        const reconnected = MockWebSocket.getLastInstance()!;
        expect(reconnected).not.toBe(ws);

        reconnected.simulateMessage({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(lastOf(batchesSentOn(reconnected))).toEqual(['1', '2', '3']);

        // Neither the lost connection nor the new authentication may forget the
        // batch: this ack answers it whichever of the sends the server processed.
        reconnected.simulateMessage(applied('3'));
        await jest.runAllTimersAsync();

        expectRetiredUpTo(3);
      });

      test('on HTTP, an empty acceptance set accepts nothing even when its lastId is the last id of a sent batch', async () => {
        const http = httpMock();
        await bootOn(http, threeOps());
        expect(http.sentBatches()).toEqual([['1', '2', '3']]);

        // Over HTTP a matching lastId alone retires a batch, which is exactly
        // why an empty acceptance set must not be read as "no results" there.
        await expectNotAVerdict(() =>
          http.deliver({ type: 'OP_ACK', payload: { lastId: '3', results: [] } }),
        );
        expect([...(inFlight()?.get('3') ?? [])]).toEqual(['1', '2', '3']);
      });

      test('a results-less ack for the last id of a sent batch retires exactly that batch', async () => {
        const ws = await bootWith(threeOps());
        expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

        ws.simulateMessage(applied('3'));
        await jest.runAllTimersAsync();

        expectRetiredUpTo(3);
      });

      test('a single-op batch is retired by the CLIENT_OP-shaped ack', async () => {
        const ws = await bootWith([pendingOp('1', 'user1')]);
        expect(batchesSentOn(ws)).toEqual([['1']]);

        // A lone CLIENT_OP and a one-op batch are answered with the same frame.
        ws.simulateMessage(applied('1'));
        await jest.runAllTimersAsync();

        expectRetiredUpTo(1);
      });

      test('two in-flight batches are each retired by their own ack, in either order', async () => {
        // Op 1 is flushed alone; a second write arrives before any answer and is
        // flushed together with the still-pending op 1.
        const bootTwoBatches = async () => {
          issueOpIdsFrom(2);
          const ws = await bootWith([pendingOp('1', 'user1')], {
            connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
          });
          await write('user2');
          expect(batchesSentOn(ws)).toEqual([['1'], ['1', '2']]);
          mockStorage.markOpsSynced.mockClear();
          return ws;
        };

        const inOrder = await bootTwoBatches();
        inOrder.simulateMessage(applied('1'));
        await jest.runAllTimersAsync();
        expect(opLogOf().map((o) => o.id)).toEqual(['2']);
        expect(mockStorage.markOpsSynced.mock.calls).toEqual([[1]]);
        inOrder.simulateMessage(applied('2'));
        await jest.runAllTimersAsync();
        expect(opLogOf()).toHaveLength(0);
        expect(mockStorage.markOpsSynced.mock.calls).toEqual([[1], [2]]);

        syncEngine!.close();

        const reversed = await bootTwoBatches();
        reversed.simulateMessage(applied('2'));
        await jest.runAllTimersAsync();
        expectRetiredUpTo(2);
        // The ack of the first batch arrives late and finds nothing left to say.
        await expectNotAVerdict(() => reversed.simulateMessage(applied('1')));
      });

      test("an ack for one node's batch does not delete a still-pending op of another node's batch", async () => {
        const cluster = clusterMock({
          nodeOf: (opId) => (Number(opId) % 2 === 1 ? 'node-a' : 'node-b'),
        });
        await bootOn(cluster, [
          pendingOp('1', 'user1'),
          pendingOp('2', 'user2'),
          pendingOp('3', 'user3'),
          pendingOp('4', 'user4'),
        ]);
        expect(cluster.frames).toEqual([
          ['1', '3'],
          ['2', '4'],
        ]);

        // Node A acknowledges its frame. Op 2 travelled to node B and nobody has
        // answered for it, though its id is below the acknowledged one.
        cluster.deliver(applied('3'));
        await jest.runAllTimersAsync();

        expect(opLogOf().map((o) => o.id)).toEqual(['2', '4']);
        expect(opLogOf().every((o) => !o.synced)).toBe(true);
        // The durable mark deletes every row at or below its id, so it has to
        // stop under the smallest op that is still pending.
        expect(mockStorage.markOpsSynced.mock.calls).toEqual([[1]]);

        cluster.deliver(applied('4'));
        await jest.runAllTimersAsync();

        expect(opLogOf()).toHaveLength(0);
        expect(mockStorage.markOpsSynced.mock.calls).toEqual([[1], [4]]);
      });

      test('two in-flight batches with the same last id are covered by their intersection', async () => {
        // Two frames ending in the same op are in flight at once and the ack does
        // not say which one it answers, so it vouches only for the ops that are
        // in both.
        const covered = async (
          ops: ReturnType<typeof pendingOp>[],
          firstFlush: string[][],
          secondFlush: string[][],
          stillPending: string[],
        ) => {
          const cluster = clusterMock();
          cluster.script(firstFlush, secondFlush);
          mockStorage.markOpsSynced.mockClear();
          await bootOn(cluster, ops);
          cluster.deliver({ type: 'AUTH_ACK' });
          await jest.runAllTimersAsync();
          expect(cluster.frames).toEqual([...firstFlush, ...secondFlush]);

          cluster.deliver(applied('3'));
          await jest.runAllTimersAsync();

          expect(opLogOf().map((o) => o.id)).toEqual(stillPending);
          expect(opLogOf().every((o) => !o.synced)).toBe(true);
          // Op 3 is retired in memory only: a smaller op is still pending, and
          // the durable mark would delete it along with everything below 3.
          expect(mockStorage.markOpsSynced).not.toHaveBeenCalled();

          // The entry was used up by the ack it matched.
          await expectNotAVerdict(() => cluster.deliver(applied('3')));
        };

        // A wider frame first, then a narrower one.
        await covered(
          [pendingOp('2', 'user2'), pendingOp('3', 'user3')],
          [['2', '3']],
          [['3']],
          ['2'],
        );

        syncEngine!.close();

        // The narrower frame first, then a wider one that reuses its last id.
        await covered(threeOps(), [['3']], [['1', '2', '3']], ['1', '2']);
      });

      test('the in-flight registry is bounded', async () => {
        await bootWith([], roomy);
        issueOpIdsFrom(1);

        // No write is ever answered, so flush n carries ops 1..n and ends in n:
        // three hundred distinct batches.
        for (const n of range(1, 300)) {
          await write(`key${n}`);
        }
        expect(syncEngine!.getPendingOpsCount()).toBe(300);

        const registry = inFlight();
        expect(registry).toBeInstanceOf(Map);

        const nonEmpty = [...registry!.values()].filter((ids) => ids.size > 0);
        expect(nonEmpty.length).toBeLessThanOrEqual(256);
        // The bound empties the oldest sets and keeps their keys: a key that was
        // dropped could be registered afresh and acknowledge ops again.
        for (const n of range(1, 44)) {
          expect(registry!.get(String(n))).toEqual(new Set());
        }
        // The oldest batch inside the bound is untouched.
        expect(registry!.get('45')).toEqual(new Set(range(1, 45).map(String)));
      });

      test('an HTTP batch re-sent by the provider after a failed poll is retired by its ack', async () => {
        const http = httpMock();
        await bootOn(http, [pendingOp('1', 'user1')]);
        expect(http.sentBatches()).toEqual([['1']]);
        const lowWaterSpy = jest.spyOn(engine().backpressureController, 'checkLowWaterMark');

        // A failed poll: the provider reports the loss, re-sends the request on
        // its own and delivers the answer. The engine sees no new connection, no
        // new authentication and makes no new flush in between.
        http.emit('disconnected', 'mock-node');
        http.deliver({ type: 'OP_ACK', payload: { lastId: '1' } });
        await jest.runAllTimersAsync();

        expectRetiredUpTo(1);
        expect(lowWaterSpy).toHaveBeenCalled();
      });

      test('a results-less OP_ACK that carries achievedLevel but answers no sent batch is not a verdict', async () => {
        const ws = await bootWith(threeOps());
        expect(batchesSentOn(ws)).toEqual([['1', '2', '3']]);

        // A level does not turn a frame into an answer: no batch ending in op 900
        // was ever sent.
        await expectNotAVerdict(() => ws.simulateMessage(applied('900')));
      });

      test('a provider whose sendBatch reports no batch is warned about once', async () => {
        const warnSpy = jest.spyOn(logger, 'warn');
        const cluster = clusterMock({ reportsBatches: false });

        await bootOn(cluster, [pendingOp('1', 'user1'), pendingOp('2', 'user2')]);
        cluster.deliver({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(cluster.sendBatch).toHaveBeenCalledTimes(2);

        // Without a report the engine cannot tell which acks answer this
        // provider's batches, so its writes retire only through acks that name
        // their ops. That has to be said, and said once rather than per flush.
        const warnings = warnSpy.mock.calls.filter((args) =>
          args.some(
            (arg) =>
              typeof arg === 'string' &&
              arg.includes('sendBatch did not report the batches it sent'),
          ),
        );
        warnSpy.mockRestore();
        expect(warnings).toHaveLength(1);
      });

      test('a provider that declares no transport keeps the registry rule alone: a matched ack without achievedLevel retires the batch', async () => {
        const bare = bareMock();
        await bootOn(bare, threeOps());
        expect('transport' in bare.provider).toBe(false);
        expect(bare.sentBatches()).toEqual([['1', '2', '3']]);

        // A provider written before the declaration existed must keep working:
        // the engine cannot ask its frames for a level it never promised.
        bare.deliver({ type: 'OP_ACK', payload: { lastId: '3' } });
        await jest.runAllTimersAsync();

        expectRetiredUpTo(3);
      });

      test('on HTTP, a non-numeric lastId with no acceptance set is not a verdict', async () => {
        const http = httpMock();
        await bootOn(http, threeOps());
        expect(http.sentBatches()).toEqual([['1', '2', '3']]);

        // The HTTP answer to a request whose last op carries no id. No batch is
        // registered under that word, so it retires nothing.
        await expectNotAVerdict(() =>
          http.deliver({ type: 'OP_ACK', payload: { lastId: 'unknown' } }),
        );
      });

      test('an evicted key acknowledges nothing, and registering it again keeps it empty', async () => {
        const ws = await bootWith([], roomy);
        issueOpIdsFrom(1);
        for (const n of range(1, 300)) {
          await write(`key${n}`);
        }
        expect(lastOf(batchesSentOn(ws))).toHaveLength(300);

        // (1) The batch that ended in op 1 is far past the bound, so its set was
        // emptied. Its late ack must retire nothing, and the key must stay: with
        // the key gone the ack could not be told from a stray one, and with the
        // set restored it would acknowledge again.
        await expectNotAVerdict(() => ws.simulateMessage(applied('1')));
        expect(inFlight()?.get('1')).toEqual(new Set());

        // (2) Every other op is retired by an ack that names it, leaving op 1 as
        // the only pending write.
        ws.simulateMessage(
          ack({
            lastId: '300',
            results: range(2, 300).map((n) => ({
              opId: String(n),
              success: true,
              achievedLevel: 'APPLIED',
            })),
          }),
        );
        await jest.runAllTimersAsync();
        expect(opLogOf().map((o) => o.id)).toEqual(['1']);

        // A reconnect flushes op 1 alone, under the very key that was emptied.
        ws.close();
        await jest.runAllTimersAsync();
        const reconnected = MockWebSocket.getLastInstance()!;
        expect(reconnected).not.toBe(ws);
        reconnected.simulateMessage({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(lastOf(batchesSentOn(reconnected))).toEqual(['1']);

        // Registering under an emptied key must not refill it.
        await expectNotAVerdict(() => reconnected.simulateMessage(applied('1')));
        expect(inFlight()?.get('1')).toEqual(new Set());

        // (3) Op 1 is not stranded: the next write flushes it under a fresh key,
        // and that batch's ack retires both.
        await write('key301');
        expect(lastOf(batchesSentOn(reconnected))).toEqual(['1', '301']);
        mockStorage.markOpsSynced.mockClear();
        reconnected.simulateMessage(applied('301'));
        await jest.runAllTimersAsync();

        expectRetiredUpTo(301);
      });

      test('the transport is read when the ack arrives, not when the engine is built', async () => {
        const late = bareMock();
        await bootOn(late, threeOps());
        expect(late.sentBatches()).toEqual([['1', '2', '3']]);

        // A provider that picks its transport while connecting has none to show
        // at construction. A value copied then would be "none declared" for the
        // whole session, and the frame below would retire the batch.
        late.provider.transport = 'websocket';

        await expectNotAVerdict(() => late.deliver({ type: 'OP_ACK', payload: { lastId: '3' } }));

        late.deliver(applied('3'));
        await jest.runAllTimersAsync();

        expectRetiredUpTo(3);
      });

      test('entries with nothing pending are pruned before an eviction, so refused writes do not grow the registry', async () => {
        const ws = await bootWith([], roomy);
        issueOpIdsFrom(1);

        // Each write is flushed alone and refused before the next one is made, so
        // every batch names a single op that has already left the op log, and no
        // ack is ever applied: nothing but the bound itself can clean up.
        for (const n of range(1, 300)) {
          await write(`key${n}`);
          ws.simulateMessage(refusal(String(n)));
          await jest.advanceTimersByTimeAsync(0);
        }
        expect(lastOf(batchesSentOn(ws))).toEqual(['300']);
        expect(opLogOf()).toHaveLength(0);
        expect(mockStorage.markOpsSynced).not.toHaveBeenCalled();

        const registry = inFlight();
        expect(registry).toBeInstanceOf(Map);

        const keys = [...registry!.keys()];
        expect(keys.length).toBeLessThanOrEqual(256);
        // Emptying the oldest set keeps its key for good when no ack ever
        // arrives, so dead entries have to be deleted before anything is emptied.
        expect([...registry!.values()].filter((ids) => ids.size === 0)).toHaveLength(0);
        // The 257th batch found 256 entries with nothing pending and deleted them
        // all; only what was recorded from then on remains.
        expect(keys).toEqual(range(257, 300).map(String));
      });

      test('a refused op that is still in the op log keeps its sent-batch entry, so a late ack cannot retire an op its batch never carried', async () => {
        // Op 1 finds no node on the first flush; op 2 and op 3 each travel alone.
        const cluster = clusterMock();
        cluster.script([['2'], ['3']]);
        await bootOn(cluster, threeOps());
        expect(cluster.frames).toEqual([['2'], ['3']]);
        expect(inFlight()?.get('2')).toEqual(new Set(['2']));

        // Op 2 is refused for good, but storage will not delete its row, so it
        // stays in the op log, flagged, and every later flush sends it again.
        mockStorage.deleteOp.mockRejectedValue(new Error('storage refused the delete'));
        cluster.deliver(refusal('2'));
        await jest.runAllTimersAsync();
        expect(opLogOf().map((o) => o.id)).toEqual(['1', '2', '3']);
        expect(opLogOf().find((o) => o.id === '2')?.rejected).toBe(true);

        // Another batch is acknowledged, which is when finished entries are
        // cleared out. Op 1 is still pending below it, so no durable row goes.
        cluster.deliver(applied('3'));
        await jest.runAllTimersAsync();
        expect(opLogOf().map((o) => o.id)).toEqual(['1', '2']);

        // Op 1 now has a node, the same one, and the flush that carries it ends
        // in the refused op again: a second, wider frame under the same last id.
        cluster.script([['1', '2']]);
        cluster.deliver({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(cluster.frames).toEqual([['2'], ['3'], ['1', '2']]);
        const recordedUnderRefusedOp = new Set(inFlight()?.get('2'));
        const pendingBefore = syncEngine!.getPendingOpsCount();

        // The answer to the FIRST frame arrives late. That frame carried op 2
        // alone, so it says nothing about op 1.
        cluster.deliver(applied('2'));
        await jest.runAllTimersAsync();

        expect(opLogOf().map((o) => o.id)).toEqual(['1', '2']);
        expect(opLogOf().find((o) => o.id === '1')?.synced).toBeFalsy();
        expect(syncEngine!.getPendingOpsCount()).toBe(pendingBefore);
        expect(mockStorage.markOpsSynced).not.toHaveBeenCalled();
        // Both frames were on record under that id, so it vouched only for the
        // op they share.
        expect(recordedUnderRefusedOp).toEqual(new Set(['2']));
      });

      test("an ack that matches a refused op's entry does not free the key while the op can still be sent", async () => {
        // Op 2 travels alone, twice, before anything is answered; op 1 finds no
        // node on either flush.
        const cluster = clusterMock();
        cluster.script([['2']], [['2']]);
        await bootOn(cluster, [pendingOp('1', 'user1'), pendingOp('2', 'user2')]);
        cluster.deliver({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(cluster.frames).toEqual([['2'], ['2']]);

        // Op 2 is refused for good, but storage will not delete its row, so it
        // stays in the op log, flagged, and every later flush sends it again.
        mockStorage.deleteOp.mockRejectedValue(new Error('storage refused the delete'));
        cluster.deliver(refusal('2'));
        await jest.runAllTimersAsync();

        // The answer to the first frame arrives late. It matches the entry and
        // has nothing to retire: the only op it covers is the refused one.
        cluster.deliver(applied('2'));
        await jest.runAllTimersAsync();
        expect(opLogOf().map((o) => o.id)).toEqual(['1', '2']);

        // Op 1 now has a node, and the frame that carries it ends in the refused
        // op again: a wider frame under the same last id.
        cluster.script([['1', '2']]);
        cluster.deliver({ type: 'AUTH_ACK' });
        await jest.runAllTimersAsync();
        expect(cluster.frames).toEqual([['2'], ['2'], ['1', '2']]);
        const recordedUnderRefusedOp = new Set(inFlight()?.get('2'));
        const pendingBefore = syncEngine!.getPendingOpsCount();

        // The answer to the SECOND frame arrives. That frame, too, carried op 2
        // alone, so it says nothing about op 1.
        cluster.deliver(applied('2'));
        await jest.runAllTimersAsync();

        expect(opLogOf().map((o) => o.id)).toEqual(['1', '2']);
        expect(opLogOf().find((o) => o.id === '1')?.synced).toBeFalsy();
        expect(syncEngine!.getPendingOpsCount()).toBe(pendingBefore);
        expect(mockStorage.markOpsSynced).not.toHaveBeenCalled();
        // The first answer did not use the entry up, so the wider frame was
        // intersected with it instead of being recorded afresh.
        expect(recordedUnderRefusedOp).toEqual(new Set(['2']));
        // And it is still there for as long as the refused op can be sent.
        expect(inFlight()?.get('2')).toEqual(new Set(['2']));
      });
    });
  });
});
