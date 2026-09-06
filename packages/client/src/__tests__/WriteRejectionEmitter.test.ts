import { WriteRejectionEmitter, writeRejectionCauseFromCode } from '../WriteRejectionEmitter';
import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import { IStorageAdapter, OpLogEntry as StoredOpLogEntry } from '../IStorageAdapter';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { serialize, deserialize } from '@topgunbuild/core';
import type { WriteRejection } from '@topgunbuild/core';
import { logger } from '../utils/logger';

// --- Mock WebSocket --------------------------------------------------------

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.OPEN;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  sentMessages: unknown[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => this.onopen?.(), 0);
  }

  send(data: Uint8Array | string) {
    this.sentMessages.push(typeof data === 'string' ? JSON.parse(data) : deserialize(data));
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: 'Normal closure' });
  }

  simulateMessage(message: unknown) {
    if (!this.onmessage) return;
    const data = serialize(message);
    this.onmessage({ data: new Uint8Array(data).buffer });
  }

  static reset() {
    MockWebSocket.instances = [];
  }

  static getLastInstance(): MockWebSocket | undefined {
    return MockWebSocket.instances[MockWebSocket.instances.length - 1];
  }
}

(global as unknown as { WebSocket: unknown }).WebSocket = MockWebSocket;
(global as unknown as { crypto: unknown }).crypto = { randomUUID: () => 'test-uuid' };

function createMockStorageAdapter(): jest.Mocked<IStorageAdapter> {
  return {
    initialize: jest.fn().mockResolvedValue(undefined),
    close: jest.fn().mockResolvedValue(undefined),
    get: jest.fn().mockResolvedValue(undefined),
    put: jest.fn().mockResolvedValue(undefined),
    remove: jest.fn().mockResolvedValue(undefined),
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
  } as unknown as jest.Mocked<IStorageAdapter>;
}

const OP_TIMESTAMP = { millis: 1000, counter: 0, nodeId: 'test-node' };

const pendingOp = (id: string, key: string, value: unknown) => ({
  id,
  mapName: 'users',
  opType: 'PUT',
  key,
  synced: false,
  record: { value, timestamp: OP_TIMESTAMP },
  timestamp: OP_TIMESTAMP,
});

const refusal = (opId: string, code = 403, permanent = true) => ({
  type: 'OP_REJECTED',
  payload: { opId, reason: 'write forbidden by policy', code, permanent },
});

// --- The emitter in isolation ----------------------------------------------

describe('WriteRejectionEmitter', () => {
  const event = (id: string): WriteRejection => ({
    id,
    mapName: 'users',
    key: 'u1',
    attemptedValue: { name: 'ada' },
    cause: 'forbidden',
    reason: 'nope',
    keptLocally: true,
    previouslyAcked: false,
    timestamp: OP_TIMESTAMP,
  });

  test('delivers to every listener and stops after unsubscribe', () => {
    const emitter = new WriteRejectionEmitter();
    const a: WriteRejection[] = [];
    const b: WriteRejection[] = [];

    const offA = emitter.subscribe((r) => a.push(r));
    emitter.subscribe((r) => b.push(r));
    expect(emitter.listenerCount).toBe(2);

    emitter.emit(event('1'));
    offA();
    emitter.emit(event('2'));

    expect(a.map((r) => r.id)).toEqual(['1']);
    expect(b.map((r) => r.id)).toEqual(['1', '2']);
    expect(emitter.listenerCount).toBe(1);
  });

  test('a listener that throws neither hides the event from the others nor escapes', () => {
    const errorSpy = jest.spyOn(logger, 'error').mockImplementation(() => undefined);
    const emitter = new WriteRejectionEmitter();
    const seen: string[] = [];

    emitter.subscribe(() => {
      throw new Error('listener blew up');
    });
    emitter.subscribe((r) => seen.push(r.id));

    expect(() => emitter.emit(event('1'))).not.toThrow();
    expect(seen).toEqual(['1']);
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  test('holds no buffer: a listener registered after the fact receives nothing', () => {
    const emitter = new WriteRejectionEmitter();
    emitter.emit(event('1'));

    const late: string[] = [];
    emitter.subscribe((r) => late.push(r.id));
    expect(late).toEqual([]);
  });

  test('clear() drops every listener', () => {
    const emitter = new WriteRejectionEmitter();
    const seen: string[] = [];
    emitter.subscribe((r) => seen.push(r.id));

    emitter.clear();
    emitter.emit(event('1'));

    expect(emitter.listenerCount).toBe(0);
    expect(seen).toEqual([]);
  });
});

describe('writeRejectionCauseFromCode', () => {
  test('maps the three per-op refusal wire codes onto the closed union', () => {
    expect(writeRejectionCauseFromCode(403)).toBe('forbidden');
    expect(writeRejectionCauseFromCode(413)).toBe('value_too_large');
    expect(writeRejectionCauseFromCode(422)).toBe('schema_invalid');
  });

  test('reports an unmapped or absent code as forbidden, and logs the unmapped one', () => {
    const warnSpy = jest.spyOn(logger, 'warn').mockImplementation(() => undefined);

    expect(writeRejectionCauseFromCode(undefined)).toBe('forbidden');
    expect(warnSpy).not.toHaveBeenCalled();

    expect(writeRejectionCauseFromCode(4003)).toBe('forbidden');
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });
});

// --- The emitter as wired into SyncEngine ----------------------------------

describe('SyncEngine write-rejection surface', () => {
  let syncEngine: SyncEngine | undefined;
  let mockStorage: jest.Mocked<IStorageAdapter>;
  let config: SyncEngineConfig;

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.reset();
    mockStorage = createMockStorageAdapter();
    config = {
      nodeId: 'test-node',
      connectionProvider: new SingleServerProvider({ url: 'ws://localhost:8080' }),
      storageAdapter: mockStorage,
      reconnectInterval: 1000,
      heartbeat: { enabled: false },
    };
  });

  afterEach(() => {
    syncEngine?.close();
    syncEngine = undefined;
    jest.useRealTimers();
    jest.clearAllMocks();
  });

  async function bootWith(ops: ReturnType<typeof pendingOp>[]) {
    mockStorage.getPendingOps.mockResolvedValue(ops as unknown as StoredOpLogEntry[]);
    syncEngine = new SyncEngine(config);
    await jest.runAllTimersAsync();
    return MockWebSocket.getLastInstance()!;
  }

  test('emits exactly one event per refused write, carrying the refused value', async () => {
    const ws = await bootWith([pendingOp('7', 'user7', { name: 'ada' })]);
    const seen: WriteRejection[] = [];
    syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('7'));
    await jest.runAllTimersAsync();

    expect(seen).toHaveLength(1);
    expect(seen[0]).toMatchObject({
      id: '7',
      mapName: 'users',
      key: 'user7',
      attemptedValue: { name: 'ada' },
      cause: 'forbidden',
      reason: 'write forbidden by policy',
      keptLocally: true,
      previouslyAcked: false,
      timestamp: OP_TIMESTAMP,
    });
  });

  test('a refusal delivered twice is emitted once', async () => {
    const ws = await bootWith([pendingOp('7', 'user7', { name: 'ada' })]);
    const seen: WriteRejection[] = [];
    syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('7'));
    ws.simulateMessage(refusal('7'));
    await jest.runAllTimersAsync();

    expect(seen).toHaveLength(1);
  });

  test('a non-permanent refusal is retried, not presented', async () => {
    const ws = await bootWith([pendingOp('7', 'user7', { name: 'ada' })]);
    const seen: WriteRejection[] = [];
    syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('7', 403, false));
    await jest.runAllTimersAsync();

    expect(seen).toEqual([]);
  });

  test('a refusal naming an op the client no longer holds is still presented, keyed by op id', async () => {
    const ws = await bootWith([]);
    const seen: WriteRejection[] = [];
    syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('99', 422));
    await jest.runAllTimersAsync();

    expect(seen).toHaveLength(1);
    // No op to read a target from: the id falls back to the op id rather than
    // to a `mapName:key:millis:counter` form built out of unknowns.
    expect(seen[0].id).toBe('99');
    expect(seen[0].mapName).toBe('');
    expect(seen[0].key).toBe('');
    expect(seen[0].attemptedValue).toBeUndefined();
    expect(seen[0].cause).toBe('schema_invalid');
    // Nothing durable was attempted for an op that is not there.
    expect(mockStorage.deleteOp).not.toHaveBeenCalled();
  });

  test('the refused record flips to conflicted for useSyncState', async () => {
    const ws = await bootWith([pendingOp('7', 'user7', { name: 'ada' })]);
    const tracker = syncEngine!.getRecordSyncStateTracker();
    expect(tracker.get('users', 'user7')).not.toBe('conflicted');

    ws.simulateMessage(refusal('7'));
    await jest.runAllTimersAsync();

    expect(tracker.get('users', 'user7')).toBe('conflicted');
  });

  test('an unknown-target refusal creates no tracker slot', async () => {
    const ws = await bootWith([]);
    const tracker = syncEngine!.getRecordSyncStateTracker();

    ws.simulateMessage(refusal('99'));
    await jest.runAllTimersAsync();

    expect(tracker.get('', '')).toBe('synced');
    expect(tracker.getMapSnapshot('').size).toBe(0);
  });

  test('a listener that throws does not break the refusal path', async () => {
    const errorSpy = jest.spyOn(logger, 'error').mockImplementation(() => undefined);
    const ws = await bootWith([pendingOp('7', 'user7', { name: 'ada' })]);
    const seen: WriteRejection[] = [];
    syncEngine!.onWriteRejected(() => {
      throw new Error('listener blew up');
    });
    syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('7'));
    await jest.runAllTimersAsync();

    expect(seen).toHaveLength(1);
    // The op was still retired despite the throwing listener.
    expect(mockStorage.deleteOp).toHaveBeenCalledWith(7);
    await expect(syncEngine!.waitForOpSynced('7', 100)).resolves.toBe('rejected');
    errorSpy.mockRestore();
  });

  test('unsubscribing stops delivery', async () => {
    const ws = await bootWith([
      pendingOp('7', 'user7', { name: 'ada' }),
      pendingOp('8', 'user8', { name: 'bob' }),
    ]);
    const seen: WriteRejection[] = [];
    const off = syncEngine!.onWriteRejected((r) => seen.push(r));

    ws.simulateMessage(refusal('7'));
    await jest.runAllTimersAsync();
    off();
    ws.simulateMessage(refusal('8'));
    await jest.runAllTimersAsync();

    expect(seen.map((r) => r.id)).toEqual(['7']);
  });
});
