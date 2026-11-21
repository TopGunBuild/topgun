/**
 * Each pending op is on the wire at most once per connection, and an op whose
 * batch got no verdict is re-sent by the resend timer: 10 s after its send, then
 * 20, 40 and every 60 s while it stays unanswered on one connection.
 *
 * Every test drives a mock connection provider whose `connected` and
 * `disconnected` events it emits itself, so a reconnect lands at an exact time
 * on the fake clock. A provider without `sendBatch` takes the engine's
 * single-server path (one OP_BATCH per planned frame through `send`); the
 * cluster mock reports each frame it puts on the wire through `onBatchSent`.
 */
import { serialize, deserialize } from '@topgunbuild/core';
import { SyncEngine, SyncEngineConfig, OpLogEntry } from '../SyncEngine';
import { IStorageAdapter, OpLogEntry as StoredOpLogEntry } from '../IStorageAdapter';

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
  };
}

interface Sent {
  /** `performance.now()` when the frame reached the provider. */
  at: number;
  ids: string[];
}

/** A connection provider whose events the test emits; every OP_BATCH it is handed is logged. */
function providerMock(extra: Record<string, unknown> = {}) {
  const handlers = new Map<string, Set<(...args: any[]) => void>>();
  const emit = (event: string, ...args: unknown[]) =>
    handlers.get(event)?.forEach((handler) => handler(...args));
  const sent: Sent[] = [];

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
    send: jest.fn().mockImplementation((data: Uint8Array) => {
      const message = deserialize(data) as any;
      if (message?.type === 'OP_BATCH') {
        sent.push({ at: performance.now(), ids: message.payload.ops.map((o: any) => o.id) });
      }
    }),
    forceReconnect: jest.fn(),
    close: jest.fn().mockResolvedValue(undefined),
    transport: 'websocket',
    ...extra,
  };

  return {
    provider,
    emit,
    sent,
    /** Every OP_BATCH handed to the provider, as op-id lists, in send order. */
    frames: () => sent.map((s) => s.ids),
    deliver: (message: unknown) =>
      emit('message', 'mock-node', new Uint8Array(serialize(message)).buffer),
    /** A connection loss and a new connection; the auth-optional grace then flushes. */
    reconnect: () => {
      emit('disconnected', 'mock-node');
      emit('connected', 'mock-node');
    },
  };
}

/**
 * Cluster provider: `sendBatch` puts the frames of each call on the wire and
 * reports each one. `script(...)` queues, per upcoming call, the exact frames
 * that call puts on the wire (an op in none of them fails); unscripted calls
 * split by `nodeOf`.
 */
function clusterMock(options: { nodeOf?: (opId: string) => string } = {}) {
  const { nodeOf = () => 'mock-node' } = options;
  const scripted: string[][][] = [];
  const sent: Sent[] = [];
  const calls: string[][] = [];

  const sendBatch = jest.fn(
    (
      operations: Array<{ key: string; message: { id: string } }>,
      onBatchSent?: (opIds: string[]) => void,
    ) => {
      calls.push(operations.map((o) => o.message.id));
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
      const ok = new Set<string>();
      for (const frame of callFrames) {
        if (frame.length === 0) continue;
        sent.push({ at: performance.now(), ids: frame });
        frame.forEach((id) => ok.add(id));
        onBatchSent?.(frame);
      }
      return new Map(operations.map((op) => [op.key, ok.has(op.message.id)]));
    },
  );

  const base = providerMock({ sendBatch });
  return {
    ...base,
    sent,
    frames: () => sent.map((s) => s.ids),
    sendBatch,
    /** The op ids the engine passed to each `sendBatch` call, in call order. */
    calls,
    script: (...callFrames: string[][][]) => {
      scripted.push(...callFrames);
    },
  };
}

const BOOT_MS = 1_000;
const RECONNECT_MS = 600;
const ack = (payload: Record<string, unknown>) => ({ type: 'OP_ACK', payload });
const applied = (lastId: string) => ack({ lastId, achievedLevel: 'APPLIED' });
const refusal = (opId: string, permanent: boolean) => ({
  type: 'OP_REJECTED',
  payload: { opId, reason: 'refused', code: 4003, permanent },
});
const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, i) => String(from + i));
const pendingOp = (id: string, key = `k${id}`) => ({
  id,
  mapName: 'users',
  opType: 'PUT',
  key,
  synced: false,
  timestamp: { millis: 1000, counter: 0, nodeId: 'test' },
});

describe('SyncEngine: each pending op is on the wire once per connection', () => {
  let engine: SyncEngine | undefined;
  let storage: jest.Mocked<IStorageAdapter>;

  beforeEach(() => {
    jest.useFakeTimers();
    storage = createMockStorageAdapter();
  });

  afterEach(() => {
    engine?.close();
    engine = undefined;
    jest.clearAllTimers();
    jest.useRealTimers();
  });

  type Privates = {
    opLog: OpLogEntry[];
    inFlightBatches: Map<string, Set<string>>;
    resendTimer: unknown;
  };
  const privates = () => engine as unknown as Privates;
  const pendingIds = () =>
    privates()
      .opLog.filter((o) => !o.synced)
      .map((o) => o.id);

  async function boot(
    mock: { provider: Record<string, any> },
    ops: ReturnType<typeof pendingOp>[] = [],
    overrides: Partial<SyncEngineConfig> = {},
  ) {
    storage.getPendingOps.mockResolvedValue(ops as unknown as StoredOpLogEntry[]);
    engine = new SyncEngine({
      nodeId: 'test-node',
      connectionProvider: mock.provider as unknown as SyncEngineConfig['connectionProvider'],
      storageAdapter: storage,
      heartbeat: { enabled: false },
      backpressure: { maxPendingOps: 100_000 },
      ...overrides,
    });
    await jest.advanceTimersByTimeAsync(BOOT_MS);
  }

  const issueOpIdsFrom = (first: number) => {
    let next = first;
    storage.appendOpLog.mockImplementation(async () => next++);
  };

  const write = (key: string) => {
    const timestamp = { millis: 2000, counter: 0, nodeId: 'test-node' };
    return engine!.recordOperation('users', 'PUT', key, {
      record: { value: { key }, timestamp },
      timestamp,
    });
  };

  const settle = () => jest.advanceTimersByTimeAsync(0);

  /** Advances the fake clock to `t` ms after `t0`, in steps no timer is skipped by. */
  const advanceTo = async (t0: number, t: number) => {
    const left = t0 + t - performance.now();
    if (left > 0) await jest.advanceTimersByTimeAsync(left);
  };

  const flat = (frames: string[][]) => frames.flat();

  test('case 1: 100 writes before any ack put 100 ops on the wire, each once (single server)', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    for (let n = 1; n <= 100; n++) await write(`key${n}`);

    const ids = flat(mock.frames());
    expect(ids).toHaveLength(100);
    expect(new Set(ids).size).toBe(100);
  });

  test('case 2: 100 writes through a reporting two-node provider put 100 ops on the wire', async () => {
    const cluster = clusterMock({ nodeOf: (id) => (Number(id) % 2 === 1 ? 'a' : 'b') });
    await boot(cluster);
    issueOpIdsFrom(1);
    for (let n = 1; n <= 100; n++) await write(`key${n}`);

    const ids = flat(cluster.frames());
    expect(ids).toHaveLength(100);
    expect(new Set(ids).size).toBe(100);
  });

  test('case 3: a write made while a batch is in flight is sent alone', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    await write('a');
    await write('b');

    expect(mock.frames()).toEqual([['1'], ['2']]);
  });

  test('case 4: ops sent singly whose acks are lost are re-sent once each at 10 s and retired in that round', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');
    await write('b');
    await write('c');
    expect(mock.frames()).toEqual([['1'], ['2'], ['3']]);

    await advanceTo(t0, 9_999);
    expect(mock.frames()).toHaveLength(3);
    await advanceTo(t0, 10_000);
    expect(mock.frames().slice(3)).toEqual([['1'], ['2'], ['3']]);

    for (const id of ['1', '2', '3']) mock.deliver(applied(id));
    await settle();
    expect(pendingIds()).toEqual([]);

    await jest.advanceTimersByTimeAsync(200_000);
    expect(mock.frames()).toHaveLength(6);
  });

  test('case 5: a transient ERROR is not a retry trigger; the timer re-sends at 10 s and that ack retires it', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');
    mock.deliver({ type: 'ERROR', payload: { code: 429, message: 'rate limit exceeded' } });
    await settle();
    expect(mock.frames()).toEqual([['1']]);

    await advanceTo(t0, 9_999);
    expect(mock.frames()).toEqual([['1']]);
    await advanceTo(t0, 10_000);
    expect(mock.frames()).toEqual([['1'], ['1']]);

    mock.deliver(applied('1'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });

  test('case 6: a failed results entry and a retryable refusal are re-sent once at 10 s; a permanent refusal never', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    for (const key of ['a', 'b', 'c', 'd']) await write(key);
    expect(mock.frames()).toEqual([['1'], ['2'], ['3'], ['4']]);

    mock.deliver(refusal('2', false));
    mock.deliver(refusal('4', true));
    mock.deliver(
      ack({
        lastId: '3',
        results: [
          { opId: '1', success: true, achievedLevel: 'APPLIED' },
          { opId: '3', success: false, error: 'transient' },
        ],
      }),
    );
    await settle();
    expect(pendingIds()).toEqual(['2', '3']);

    await advanceTo(t0, 10_000);
    expect(mock.frames().slice(4)).toEqual([['2'], ['3']]);
    mock.deliver(applied('2'));
    mock.deliver(applied('3'));
    await settle();
    expect(pendingIds()).toEqual([]);

    await jest.advanceTimersByTimeAsync(200_000);
    expect(mock.frames()).toHaveLength(6);
    expect(flat(mock.frames()).filter((id) => id === '4')).toHaveLength(1);
  });

  test('case 7: 300 unanswered writes are all retired by their acks, in send order and in reverse', async () => {
    for (const order of ['send', 'reverse'] as const) {
      engine?.close();
      storage = createMockStorageAdapter();
      const violations: string[] = [];
      storage.markOpsSynced.mockImplementation(async (upTo: number) => {
        for (const id of pendingIds()) {
          if (Number(id) <= upTo) violations.push(`${upTo} reaches pending ${id}`);
        }
      });
      const mock = providerMock();
      await boot(mock);
      issueOpIdsFrom(1);
      for (let n = 1; n <= 300; n++) await write(`key${n}`);
      expect(pendingIds()).toHaveLength(300);

      const ids = range(1, 300);
      for (const id of order === 'send' ? ids : [...ids].reverse()) {
        mock.deliver(applied(id));
        await settle();
      }

      expect(pendingIds()).toEqual([]);
      expect(violations).toEqual([]);
      const calls = storage.markOpsSynced.mock.calls;
      expect(calls[calls.length - 1]).toEqual([300]);
    }
  });

  test('case 8: an op whose ack is ignored is re-sent alone at 10 s under its own key and retired', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');

    // A WebSocket ack without a level is not a verdict (TG-SYNC-004).
    mock.deliver(ack({ lastId: '1' }));
    await settle();
    expect(pendingIds()).toEqual(['1']);

    await advanceTo(t0, 10_000);
    expect(mock.frames()).toEqual([['1'], ['1']]);
    mock.deliver(applied('1'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });

  test('case 9: overlapping entries keep the registry within 2P entries and 4P ids', async () => {
    const mock = providerMock();
    await boot(
      mock,
      range(1, 50).map((id) => pendingOp(id)),
    );
    expect(mock.frames()).toEqual([range(1, 50)]);

    const expectBounded = () => {
      const registry = privates().inFlightBatches;
      const p = pendingIds().length;
      const ids = [...registry.values()].reduce((sum, set) => sum + set.size, 0);
      expect(registry.size).toBeLessThanOrEqual(2 * p);
      expect(ids).toBeLessThanOrEqual(4 * p);
      for (const [key, set] of registry) expect(set.has(key)).toBe(true);
    };

    for (let r = 0; r <= 39; r++) {
      const k = 50 - r;
      expect(mock.frames()[mock.frames().length - 1]).toEqual(range(1, k));
      // Accepts only the frame's last op; results acks leave the registry alone.
      mock.deliver(
        ack({
          lastId: String(k),
          achievedLevel: 'APPLIED',
          results: [
            { opId: String(k), success: true, achievedLevel: 'APPLIED' },
            ...range(1, k - 1).map((opId) => ({ opId, success: false, error: 'retry' })),
          ],
        }),
      );
      await settle();
      expectBounded();

      mock.reconnect();
      await jest.advanceTimersByTimeAsync(RECONNECT_MS);
      expect(mock.frames()[mock.frames().length - 1]).toEqual(range(1, k - 1));
    }
  });

  test('case 10: frames {3} then {1,2,3} under one id; op 3 retires at once, ops 1 and 2 in the first re-send round', async () => {
    const cluster = clusterMock();
    cluster.script([['3']], [['1', '2', '3']]);
    await boot(cluster, [pendingOp('1'), pendingOp('2'), pendingOp('3')]);
    cluster.deliver({ type: 'AUTH_ACK' });
    await settle();
    const t1 = performance.now();
    expect(cluster.frames()).toEqual([['3'], ['1', '2', '3']]);

    cluster.deliver(applied('3'));
    cluster.deliver(applied('3'));
    await settle();
    expect(pendingIds()).toEqual(['1', '2']);

    await advanceTo(t1, 10_000);
    expect(cluster.frames().slice(2)).toEqual([['1', '2']]);
    cluster.deliver(applied('2'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });

  test("case 11: one node's frame is not handed off; its ops go with the next write, or with the timer", async () => {
    const nodeOf = (id: string) => (Number(id) % 2 === 1 ? 'a' : 'b');
    const ops = () => range(1, 4).map((id) => pendingOp(id));

    const withWrite = clusterMock({ nodeOf });
    withWrite.script([['1', '3']]);
    await boot(withWrite, ops());
    expect(withWrite.frames()).toEqual([['1', '3']]);
    issueOpIdsFrom(5);
    await write('k5');
    expect(flat(withWrite.frames().slice(1)).sort()).toEqual(['2', '4', '5']);

    engine!.close();

    const withTimer = clusterMock({ nodeOf });
    withTimer.script([['1', '3']]);
    await boot(withTimer, ops());
    const t0 = withTimer.sent[0].at;
    await advanceTo(t0, 9_999);
    expect(withTimer.frames()).toEqual([['1', '3']]);
    await advanceTo(t0, 10_000);
    const resent = flat(withTimer.frames().slice(1));
    expect(resent).toEqual(expect.arrayContaining(['2', '4']));
  });

  test('case 12: after a reconnect every op sent singly goes out once on the new connection and its acks retire all', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    for (const key of ['a', 'b', 'c']) await write(key);
    expect(mock.frames()).toEqual([['1'], ['2'], ['3']]);

    mock.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    expect(mock.frames().slice(3)).toEqual([['1'], ['2'], ['3']]);

    for (const id of ['1', '2', '3']) mock.deliver(applied(id));
    await settle();
    expect(pendingIds()).toEqual([]);

    await jest.advanceTimersByTimeAsync(200_000);
    expect(mock.frames()).toHaveLength(6);
  });

  test('case 13: a second AUTH_ACK on the same connection sends nothing already on the wire', async () => {
    const mock = providerMock();
    await boot(mock, [pendingOp('1'), pendingOp('2'), pendingOp('3')]);
    expect(mock.frames()).toEqual([['1', '2', '3']]);

    mock.deliver({ type: 'AUTH_ACK' });
    await settle();
    expect(mock.frames()).toEqual([['1', '2', '3']]);
  });

  test('case 14: the timer sends nothing while unauthenticated; the next authentication sends the ops', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');
    expect(mock.frames()).toEqual([['1']]);

    mock.emit('disconnected', 'mock-node');
    await advanceTo(t0, 15_000);
    expect(mock.frames()).toEqual([['1']]);

    mock.emit('connected', 'mock-node');
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    expect(mock.frames()).toEqual([['1'], ['1']]);
  });

  test('case 15: ops restored after authentication are sent once without a write', async () => {
    let restore: (ops: StoredOpLogEntry[]) => void = () => undefined;
    storage.getPendingOps.mockImplementation(
      () => new Promise<StoredOpLogEntry[]>((resolve) => (restore = resolve)),
    );
    const mock = providerMock();
    engine = new SyncEngine({
      nodeId: 'test-node',
      connectionProvider: mock.provider as unknown as SyncEngineConfig['connectionProvider'],
      storageAdapter: storage,
      heartbeat: { enabled: false },
    });
    await jest.advanceTimersByTimeAsync(BOOT_MS);
    expect(mock.frames()).toEqual([]);

    restore([pendingOp('1'), pendingOp('2'), pendingOp('3')] as unknown as StoredOpLogEntry[]);
    await settle();
    expect(mock.frames()).toEqual([['1', '2', '3']]);
    await jest.advanceTimersByTimeAsync(5_000);
    expect(mock.frames()).toEqual([['1', '2', '3']]);
  });

  test('case 16: single-op frames acked out of order mark the durable prefix [[1], [3]]', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    for (const key of ['a', 'b', 'c']) await write(key);

    for (const id of ['3', '1', '2']) {
      mock.deliver(applied(id));
      await settle();
    }
    expect(pendingIds()).toEqual([]);
    expect(storage.markOpsSynced.mock.calls).toEqual([[1], [3]]);
  });

  test('case 16b: an accepted id no sent op carries never raises the durable mark', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    await write('a');
    await write('b');

    mock.deliver(ack({ lastId: '999', results: [{ opId: '999', success: true }] }));
    await settle();
    mock.deliver(applied('1'));
    await settle();
    mock.deliver(applied('2'));
    await settle();

    expect(pendingIds()).toEqual([]);
    expect(storage.markOpsSynced.mock.calls).toEqual([[1], [2]]);
  });

  test('case 17: close() with ops on the wire clears the timer and sends nothing after', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    await write('a');
    expect(privates().resendTimer).not.toBeNull();

    engine!.close();
    expect(privates().resendTimer).toBeNull();
    await jest.advanceTimersByTimeAsync(200_000);
    expect(mock.frames()).toEqual([['1']]);
  });

  test('case 18: a timer that fires before its op is due re-arms, and the op is re-sent when due', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');

    await advanceTo(t0, 5_000);
    mock.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    expect(mock.frames()).toEqual([['1'], ['1']]);
    const resentAt = mock.sent[1].at;

    // The timer armed for the first send fires at 10 s and finds nothing due.
    await advanceTo(resentAt, 9_999);
    expect(mock.frames()).toHaveLength(2);
    await advanceTo(resentAt, 10_000);
    expect(mock.frames()).toEqual([['1'], ['1'], ['1']]);

    mock.deliver(applied('1'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });

  test('case 19: a reconnect moves an armed timer earlier', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');

    await advanceTo(t0, 40_000);
    expect(mock.sent.map((s) => s.at - t0)).toEqual([0, 10_000, 30_000]);
    mock.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    const reconnectAt = mock.sent[3].at - t0;

    await advanceTo(t0, 60_000);
    expect(mock.sent.map((s) => s.at - t0)).toEqual([
      0,
      10_000,
      30_000,
      reconnectAt,
      reconnectAt + 10_000,
    ]);
  });

  test("case 20: an op that is one entry's key and another's member goes in exactly one frame", async () => {
    const cluster = clusterMock({ nodeOf: (id) => (id === '3' ? 'b' : 'a') });
    cluster.script([['1', '2', '3']], [['1', '2']]);
    await boot(cluster, [pendingOp('1'), pendingOp('2'), pendingOp('3')]);
    const t0 = cluster.sent[0].at;
    expect(cluster.frames()).toEqual([['1', '2', '3']]);

    await advanceTo(t0, 10_000);
    expect(cluster.calls).toEqual([
      ['1', '2', '3'],
      ['1', '2', '3'],
    ]);
    expect(cluster.frames()).toEqual([
      ['1', '2', '3'],
      ['1', '2'],
    ]);

    await advanceTo(t0, 12_000);
    cluster.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    expect(cluster.calls.slice(2)).toEqual([['1', '2'], ['3']]);
    expect(cluster.frames().slice(2)).toEqual([['1', '2'], ['3']]);

    cluster.deliver(applied('2'));
    cluster.deliver(applied('3'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });

  test('case 21: backoff — sent at 0, 10, 30, 70, 130, 190 s; a reconnect restarts the schedule', async () => {
    const mock = providerMock();
    await boot(mock);
    issueOpIdsFrom(1);
    const t0 = performance.now();
    await write('a');

    await advanceTo(t0, 200_000);
    expect(mock.sent.map((s) => s.at - t0)).toEqual([0, 10_000, 30_000, 70_000, 130_000, 190_000]);

    mock.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    const reconnectAt = mock.sent[6].at - t0;
    await advanceTo(t0, 215_000);
    expect(mock.sent.map((s) => s.at - t0)).toEqual([
      0,
      10_000,
      30_000,
      70_000,
      130_000,
      190_000,
      reconnectAt,
      reconnectAt + 10_000,
    ]);
  });

  test('case 22: a sendable key held as a member of a newer entry keeps its key, and is retired', async () => {
    const setup = async () => {
      const cluster = clusterMock();
      cluster.script([['3']], [['3', '4']]);
      await boot(cluster);
      issueOpIdsFrom(3);
      const t0 = performance.now();
      await write('k3');
      await write('k4');
      await write('k5');
      expect(cluster.frames()).toEqual([['3'], ['3', '4'], ['5']]);
      // The ack of op 5 runs a prune; the ack of [3, 4] is lost.
      cluster.deliver(applied('5'));
      await settle();
      expect(pendingIds()).toEqual(['3', '4']);
      return { cluster, t0 };
    };

    const timer = await setup();
    await advanceTo(timer.t0, 10_000);
    expect(timer.cluster.frames().slice(3)).toEqual([['4']]);
    timer.cluster.deliver(applied('4'));
    await settle();
    await advanceTo(timer.t0, 20_000);
    expect(timer.cluster.frames().slice(4)).toEqual([['3']]);
    timer.cluster.deliver(applied('3'));
    await settle();
    expect(pendingIds()).toEqual([]);

    engine!.close();

    const reconnect = await setup();
    reconnect.cluster.reconnect();
    await jest.advanceTimersByTimeAsync(RECONNECT_MS);
    expect(reconnect.cluster.frames().slice(3)).toEqual([['3'], ['4']]);
    reconnect.cluster.deliver(applied('3'));
    reconnect.cluster.deliver(applied('4'));
    await settle();
    expect(pendingIds()).toEqual([]);
  });
});
