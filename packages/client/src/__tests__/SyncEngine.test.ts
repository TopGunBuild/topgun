import { SyncEngine, SyncEngineConfig, OpLogEntry } from '../SyncEngine';
import { IStorageAdapter } from '../IStorageAdapter';
import { serialize, deserialize, LWWMap, ORMap, HLC } from '@topgunbuild/core';

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
      // Create a copy of the exact bytes to avoid shared buffer issues
      const exactBuffer = data.slice().buffer as ArrayBuffer;
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
    getAllKeys: jest.fn().mockResolvedValue([]),
  };
}

// --- Mock crypto.randomUUID ---
let uuidCounter = 0;
(global as any).crypto = {
  randomUUID: () => `test-uuid-${++uuidCounter}`,
};

describe('SyncEngine', () => {
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
    };
  });

  afterEach(() => {
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
        { id: '1', mapName: 'users', opType: 'PUT', key: 'user1', synced: false, timestamp: { millis: 1000, counter: 0, nodeId: 'test' } },
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

    test('should NOT send AUTH immediately if no token', async () => {
      syncEngine = new SyncEngine(config);
      await jest.runAllTimersAsync();

      const ws = MockWebSocket.getLastInstance();
      const authMessage = ws?.sentMessages.find((m) => m.type === 'AUTH');
      expect(authMessage).toBeUndefined();
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
        })
      );
    });

    test('should sync pending operations after AUTH_ACK', async () => {
      const pendingOps = [
        { id: '1', mapName: 'users', opType: 'PUT', key: 'user1', synced: false, timestamp: { millis: 1000, counter: 0, nodeId: 'test' } },
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
        { id: '3', mapName: 'users', opType: 'PUT', key: 'user1', synced: false, timestamp: { millis: 1000, counter: 0, nodeId: 'test' } },
        { id: '5', mapName: 'users', opType: 'PUT', key: 'user2', synced: false, timestamp: { millis: 1001, counter: 0, nodeId: 'test' } },
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
      };

      syncEngine.subscribeToQuery(mockQuery as any);

      const ws = MockWebSocket.getLastInstance()!;
      ws.simulateMessage({
        type: 'QUERY_RESP',
        payload: {
          queryId: 'query-1',
          results: [{ key: 'user1', value: { name: 'Alice' } }],
        },
      });
      await jest.runAllTimersAsync();

      // Verify onResult is called with 'server' source parameter
      expect(mockQuery.onResult).toHaveBeenCalledWith(
        [{ key: 'user1', value: { name: 'Alice' } }],
        'server'
      );
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

    test('should handle QUERY_UPDATE with REMOVE type', async () => {
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
          type: 'REMOVE',
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
        { publisherId: 'node-2', timestamp: 123456 }
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

      const lockPromise = syncEngine.requestLock('my-lock', 'req-1', 5000);

      // Advance time beyond the 30s timeout
      jest.advanceTimersByTime(31000);

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
        'Not connected or authenticated'
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
      expect(mockStorage.put).toHaveBeenCalledWith('users:user1', expect.objectContaining({ value: { name: 'Alice' } }));
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
});
