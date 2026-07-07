import { SyncEngine, SyncEngineConfig, OpLogEntry } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize, LWWMap, ORMap, HLC } from '@topgunbuild/core';
import { SingleServerProvider } from '../connection/SingleServerProvider';

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
      ws.simulateMessage({ type: 'OP_ACK', payload: { lastId: '5' } });
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
    async function startEngineWithMap() {
      syncEngine = new SyncEngine(config);
      const hlc = syncEngine.getHLC();
      const lwwMap = new LWWMap<string, any>(hlc);
      syncEngine.registerMap('users', lwwMap);
      await jest.runAllTimersAsync();
      const ws = MockWebSocket.getLastInstance()!;
      ws.sentMessages = []; // clear the auth/hello handshake frames
      return ws;
    }

    function batchEvent(key: string, epoch?: number) {
      return {
        mapName: 'users',
        eventType: 'PUT' as const,
        key,
        record: {
          value: { k: key },
          timestamp: { millis: Date.now(), counter: 1, nodeId: 'remote' },
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
  });
});
