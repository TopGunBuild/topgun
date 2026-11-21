import { HLC, serialize, deserialize } from '@topgunbuild/core';
import { HttpSyncProvider } from '../connection/HttpSyncProvider';
import { SyncEngine, SyncEngineConfig } from '../SyncEngine';
import type { IStorageAdapter } from '../IStorageAdapter';
// A msgpack frame as the provider emits it: a tagged message with a payload.
interface WireFrame {
  type: string;
  payload: Record<string, unknown>;
}

describe('HttpSyncProvider', () => {
  let hlc: HLC;
  let mockFetch: jest.Mock;
  let provider: HttpSyncProvider;

  function createMockResponse(body: any, status: number = 200): Response {
    const bodyBytes = serialize(body);
    return {
      ok: status >= 200 && status < 300,
      status,
      statusText: status === 200 ? 'OK' : 'Error',
      headers: new Headers({ 'content-type': 'application/x-msgpack' }),
      arrayBuffer: () =>
        Promise.resolve(
          bodyBytes.buffer.slice(bodyBytes.byteOffset, bodyBytes.byteOffset + bodyBytes.byteLength),
        ),
    } as unknown as Response;
  }

  function defaultSyncResponse(overrides: any = {}): any {
    return {
      serverHlc: { millis: Date.now(), counter: 1, nodeId: 'server-1' },
      ...overrides,
    };
  }

  beforeEach(() => {
    hlc = new HLC('client-node');
    mockFetch = jest.fn().mockResolvedValue(createMockResponse(defaultSyncResponse()));

    provider = new HttpSyncProvider({
      url: 'http://localhost:8080',
      clientId: 'test-client',
      hlc,
      authToken: 'test-token',
      pollIntervalMs: 100,
      requestTimeoutMs: 5000,
      syncMaps: ['users'],
      fetchImpl: mockFetch,
    });
  });

  afterEach(async () => {
    await provider.close();
  });

  it('connect() sends initial sync request', async () => {
    await provider.connect();

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(mockFetch).toHaveBeenCalledWith(
      'http://localhost:8080/sync',
      expect.objectContaining({
        method: 'POST',
        headers: expect.objectContaining({
          'Content-Type': 'application/x-msgpack',
          Authorization: 'Bearer test-token',
        }),
      }),
    );
  });

  it('send() queues OP_BATCH operations for next poll', async () => {
    await provider.connect();

    const opBatchMsg = serialize({
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            mapName: 'users',
            key: 'user-1',
            record: { value: { name: 'Alice' }, timestamp: hlc.now() },
          },
        ],
      },
    });

    provider.send(opBatchMsg);

    // Wait for next poll cycle
    await new Promise((r) => setTimeout(r, 150));

    // The second call (first poll after connect) should include the operations
    expect(mockFetch).toHaveBeenCalledTimes(2);
    const secondCallBody = mockFetch.mock.calls[1][1].body;
    const parsed = deserialize<any>(new Uint8Array(secondCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
  });

  it('send() silently ignores AUTH messages', async () => {
    await provider.connect();

    const authMsg = serialize({ type: 'AUTH', token: 'some-token' });
    provider.send(authMsg);

    // No error thrown, no operations queued
    await new Promise((r) => setTimeout(r, 150));

    // Poll should not include any operations
    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeUndefined();
  });

  it('send() silently ignores SYNC_INIT messages', async () => {
    await provider.connect();

    const syncInitMsg = serialize({ type: 'SYNC_INIT', mapName: 'users' });
    provider.send(syncInitMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeUndefined();
  });

  it('send() queues QUERY_SUB as one-shot query', async () => {
    await provider.connect();

    const querySubMsg = serialize({
      type: 'QUERY_SUB',
      payload: {
        requestId: 'q-1',
        mapName: 'users',
        query: { where: { active: true } },
      },
    });
    provider.send(querySubMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.queries).toBeDefined();
    expect(parsed.queries.length).toBe(1);
    expect(parsed.queries[0].queryId).toBe('q-1');
  });

  it('send() queues CLIENT_OP operations for next poll', async () => {
    await provider.connect();

    const clientOpMsg = serialize({
      type: 'CLIENT_OP',
      payload: {
        mapName: 'users',
        key: 'user-2',
        record: { value: { name: 'Bob' }, timestamp: hlc.now() },
      },
    });
    provider.send(clientOpMsg);

    await new Promise((r) => setTimeout(r, 150));

    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
    expect(parsed.operations[0].key).toBe('user-2');
  });

  it('polling loop sends queued ops at interval', async () => {
    await provider.connect();
    expect(mockFetch).toHaveBeenCalledTimes(1);

    // Wait for several poll cycles
    await new Promise((r) => setTimeout(r, 350));

    // Should have made multiple calls (initial + polls)
    expect(mockFetch.mock.calls.length).toBeGreaterThanOrEqual(3);
  });

  it('emits message events for deltas received', async () => {
    const messageHandler = jest.fn();
    provider.on('message', messageHandler);

    mockFetch.mockResolvedValue(
      createMockResponse(
        defaultSyncResponse({
          deltas: [
            {
              mapName: 'users',
              records: [
                {
                  key: 'user-1',
                  record: { value: { name: 'Alice' }, timestamp: hlc.now() },
                  eventType: 'PUT',
                },
              ],
              serverSyncTimestamp: hlc.now(),
            },
          ],
        }),
      ),
    );

    await provider.connect();

    // One SERVER_EVENT message for the delta record
    expect(messageHandler).toHaveBeenCalled();
  });

  it('translates only errors[] entries naming an operation this request sent', async () => {
    // A /sync response carries both halves of the request. The queries half has
    // written a query id into `errors[].context` since long before per-op
    // refusals existed, so a provider that branches on presence alone reports a
    // denied READ as a refusal of a write that was never refused.
    const frames: WireFrame[] = [];
    provider.on('message', (_nodeId: string, data: ArrayBuffer | Uint8Array) => {
      frames.push(
        deserialize<WireFrame>(data instanceof ArrayBuffer ? new Uint8Array(data) : data),
      );
    });

    await provider.connect();

    provider.send(
      serialize({
        type: 'OP_BATCH',
        payload: {
          ops: [
            {
              id: '7',
              mapName: 'users',
              key: 'user-7',
              record: { value: { name: 'Alice' }, timestamp: hlc.now() },
            },
          ],
        },
      }),
    );
    provider.send(
      serialize({
        type: 'QUERY_SUB',
        payload: { requestId: 'q-1788000000000', mapName: 'users', query: { where: {} } },
      }),
    );

    frames.length = 0;
    mockFetch.mockResolvedValue(
      createMockResponse(
        defaultSyncResponse({
          errors: [
            { code: 403, message: 'blocked by policy', context: '7' },
            { code: 403, message: 'access denied', context: 'q-1788000000000' },
            { code: 400, message: 'invalid cursor', context: 'q-1788000000000' },
            { code: 503, message: 'overloaded' },
          ],
          ack: { lastId: '7', results: [{ opId: '7', success: true }] },
        }),
      ),
    );

    await new Promise((r) => setTimeout(r, 150));

    const rejected = frames.filter((f) => f.type === 'OP_REJECTED');
    expect(rejected).toHaveLength(1);
    expect(rejected[0].payload.opId).toBe('7');
    expect(rejected[0].payload.permanent).toBe(true);
    expect(rejected[0].payload.code).toBe(403);
    expect(rejected[0].payload.reason).toBe('blocked by policy');

    // The query-attributed entries and the batch-level one are left exactly as
    // they were before per-op refusals existed: no frame at all, so nothing
    // downstream can retire or register them.
    const rejectedIds = rejected.map((f) => f.payload.opId);
    expect(rejectedIds).not.toContain('q-1788000000000');

    // A refusal and an acknowledgement naming the same op can share one
    // response. The refusal is emitted FIRST, which is what lets the sticky
    // check downstream keep the ack from flipping an op already refused.
    const rejectedIndex = frames.findIndex((f) => f.type === 'OP_REJECTED');
    const ackIndex = frames.findIndex((f) => f.type === 'OP_ACK');
    expect(ackIndex).toBeGreaterThan(-1);
    expect(rejectedIndex).toBeLessThan(ackIndex);
  });

  it('never retires an id claimed by both the operations and the queries half', async () => {
    // Op ids and query ids are disjoint by convention, not by type. An id in
    // both sets is ambiguous, so it is warned about and never translated.
    const frames: WireFrame[] = [];
    provider.on('message', (_nodeId: string, data: ArrayBuffer | Uint8Array) => {
      frames.push(
        deserialize<WireFrame>(data instanceof ArrayBuffer ? new Uint8Array(data) : data),
      );
    });

    await provider.connect();

    provider.send(
      serialize({
        type: 'OP_BATCH',
        payload: {
          ops: [
            {
              id: 'collide',
              mapName: 'users',
              key: 'user-9',
              record: { value: { name: 'Bob' }, timestamp: hlc.now() },
            },
          ],
        },
      }),
    );
    provider.send(
      serialize({
        type: 'QUERY_SUB',
        payload: { requestId: 'collide', mapName: 'users', query: { where: {} } },
      }),
    );

    frames.length = 0;
    mockFetch.mockResolvedValue(
      createMockResponse(
        defaultSyncResponse({
          errors: [{ code: 403, message: 'access denied', context: 'collide' }],
        }),
      ),
    );

    await new Promise((r) => setTimeout(r, 150));

    expect(frames.filter((f) => f.type === 'OP_REJECTED')).toHaveLength(0);
  });

  it('emits connected after first successful request', async () => {
    const connectedHandler = jest.fn();
    provider.on('connected', connectedHandler);

    await provider.connect();

    expect(connectedHandler).toHaveBeenCalledWith('http');
  });

  it('emits disconnected on request failure', async () => {
    const disconnectedHandler = jest.fn();
    provider.on('disconnected', disconnectedHandler);

    await provider.connect();

    // Make next request fail
    mockFetch.mockRejectedValue(new Error('Network error'));

    // Wait for poll cycle
    await new Promise((r) => setTimeout(r, 150));

    expect(disconnectedHandler).toHaveBeenCalledWith('http');
  });

  it('emits reconnected when requests succeed again after failure', async () => {
    const reconnectedHandler = jest.fn();
    provider.on('reconnected', reconnectedHandler);

    await provider.connect();

    // Make request fail
    mockFetch.mockRejectedValueOnce(new Error('Network error'));
    await new Promise((r) => setTimeout(r, 150));

    // Make request succeed again
    mockFetch.mockResolvedValue(createMockResponse(defaultSyncResponse()));
    await new Promise((r) => setTimeout(r, 150));

    expect(reconnectedHandler).toHaveBeenCalledWith('http');
  });

  it('isConnected() reflects last request status', async () => {
    expect(provider.isConnected()).toBe(false);

    await provider.connect();
    expect(provider.isConnected()).toBe(true);

    // Simulate failure
    mockFetch.mockRejectedValueOnce(new Error('fail'));
    await new Promise((r) => setTimeout(r, 150));

    expect(provider.isConnected()).toBe(false);
  });

  it('custom fetch implementation is used', async () => {
    const customFetch = jest.fn().mockResolvedValue(createMockResponse(defaultSyncResponse()));
    const customProvider = new HttpSyncProvider({
      url: 'http://custom:9999',
      clientId: 'c1',
      hlc,
      authToken: 'token',
      fetchImpl: customFetch,
    });

    await customProvider.connect();

    expect(customFetch).toHaveBeenCalled();
    expect(customFetch.mock.calls[0][0]).toBe('http://custom:9999/sync');

    await customProvider.close();
  });

  it('getConnectedNodes() returns [http] when connected and [] when disconnected', async () => {
    expect(provider.getConnectedNodes()).toEqual([]);

    await provider.connect();
    expect(provider.getConnectedNodes()).toEqual(['http']);

    await provider.close();
    expect(provider.getConnectedNodes()).toEqual([]);
  });

  it('close() stops polling loop and clears queued operations', async () => {
    await provider.connect();

    // Queue some operations
    provider.send(
      serialize({
        type: 'CLIENT_OP',
        payload: { mapName: 'users', key: 'k1', record: { value: {}, timestamp: hlc.now() } },
      }),
    );

    await provider.close();

    const callCountAtClose = mockFetch.mock.calls.length;

    // Wait to verify no more polls happen
    await new Promise((r) => setTimeout(r, 200));

    expect(mockFetch.mock.calls.length).toBe(callCountAtClose);
    expect(provider.isConnected()).toBe(false);
  });

  it('getConnection() returns a valid IConnection', () => {
    const conn = provider.getConnection('key');
    expect(conn).toBeDefined();
    expect(typeof conn.send).toBe('function');
    expect(typeof conn.close).toBe('function');
    expect(typeof conn.readyState).toBe('number');
  });

  it('getAnyConnection() returns a valid IConnection', () => {
    const conn = provider.getAnyConnection();
    expect(conn).toBeDefined();
    expect(typeof conn.send).toBe('function');
    expect(typeof conn.close).toBe('function');
    expect(typeof conn.readyState).toBe('number');
  });

  it('getConnection() readyState reflects connected state', async () => {
    // Before connect: should be CLOSED
    const connBefore = provider.getConnection('key');
    expect(connBefore.readyState).toBe(3); // CLOSED

    await provider.connect();

    // After connect: should be OPEN
    const connAfter = provider.getConnection('key');
    expect(connAfter.readyState).toBe(1); // OPEN
  });

  it('getConnection().send() queues operations like provider.send()', async () => {
    await provider.connect();

    const conn = provider.getConnection('key');
    const opBatchMsg = serialize({
      type: 'CLIENT_OP',
      payload: {
        mapName: 'users',
        key: 'user-x',
        record: { value: { name: 'Test' }, timestamp: hlc.now() },
      },
    });
    conn.send(opBatchMsg);

    // Wait for poll cycle to flush
    await new Promise((r) => setTimeout(r, 150));

    // The poll call should include the queued operation
    const lastCallBody = mockFetch.mock.calls[mockFetch.mock.calls.length - 1][1].body;
    const parsed = deserialize<any>(new Uint8Array(lastCallBody));
    expect(parsed.operations).toBeDefined();
    expect(parsed.operations.length).toBe(1);
  });

  describe('a results-less ack of a whole request is delivered per engine frame', () => {
    const op = (id: string) => ({
      id,
      mapName: 'users',
      key: `user-${id}`,
      record: { value: { id }, timestamp: hlc.now() },
    });
    const sendFrame = (...ids: string[]) =>
      provider.send(serialize({ type: 'OP_BATCH', payload: { ops: ids.map(op) } }));

    /** Every frame the provider emitted towards the engine. */
    const listen = () => {
      const frames: WireFrame[] = [];
      provider.on('message', (_nodeId: string, data: ArrayBuffer | Uint8Array) => {
        frames.push(
          deserialize<WireFrame>(data instanceof ArrayBuffer ? new Uint8Array(data) : data),
        );
      });
      return frames;
    };

    /** The op ids of each request that carried operations, in request order. */
    const requestOps = () =>
      mockFetch.mock.calls
        .map(([, init]) => deserialize<any>(new Uint8Array(init.body)).operations)
        .filter((ops: unknown) => Array.isArray(ops))
        .map((ops: Array<{ id: string }>) => ops.map((o) => o.id));

    /** Answers a request that carries operations with `answer(ids)`, any other with nothing. */
    const respond = (answer: (ids: string[]) => any) =>
      mockFetch.mockImplementation(async (_url: string, init: { body: ArrayBuffer }) => {
        const ops = deserialize<any>(new Uint8Array(init.body)).operations;
        if (!Array.isArray(ops)) return createMockResponse(defaultSyncResponse());
        return createMockResponse(
          defaultSyncResponse(answer(ops.map((o: { id: string }) => o.id))),
        );
      });

    const acks = (frames: WireFrame[]) =>
      frames.filter((f) => f.type === 'OP_ACK').map((f) => f.payload);

    it('(a) splits the ack into one OP_ACK per queued frame, in order, after any OP_REJECTED', async () => {
      await provider.connect();
      const frames = listen();
      respond((ids) => ({
        errors: [{ code: 403, message: 'blocked', context: '2' }],
        ack: { lastId: ids[ids.length - 1] },
      }));

      sendFrame('1', '2');
      sendFrame('3');
      sendFrame('4', '5', '6');
      await new Promise((r) => setTimeout(r, 150));

      expect(requestOps()).toEqual([['1', '2', '3', '4', '5', '6']]);
      expect(acks(frames)).toEqual([{ lastId: '2' }, { lastId: '3' }, { lastId: '6' }]);
      expect(frames.map((f) => f.type)).toEqual(['OP_REJECTED', 'OP_ACK', 'OP_ACK', 'OP_ACK']);
    });

    it('(d) keeps every other field of the ack on each frame ack', async () => {
      await provider.connect();
      const frames = listen();
      respond((ids) => ({
        ack: { lastId: ids[ids.length - 1], achievedLevel: 'APPLIED', extra: 7 },
      }));

      sendFrame('1');
      sendFrame('2', '3');
      await new Promise((r) => setTimeout(r, 150));

      expect(acks(frames)).toEqual([
        { lastId: '1', achievedLevel: 'APPLIED', extra: 7 },
        { lastId: '3', achievedLevel: 'APPLIED', extra: 7 },
      ]);
    });

    it('(b) forwards any other ack unchanged, as one frame', async () => {
      await provider.connect();
      const frames = listen();

      respond(() => ({ ack: { lastId: 'unknown' } }));
      sendFrame('1');
      sendFrame('2');
      await new Promise((r) => setTimeout(r, 150));
      expect(acks(frames)).toEqual([{ lastId: 'unknown' }]);

      frames.length = 0;
      const results = [
        { opId: '3', success: true },
        { opId: '4', success: true },
      ];
      respond(() => ({ ack: { lastId: '4', results } }));
      sendFrame('3');
      sendFrame('4');
      await new Promise((r) => setTimeout(r, 150));
      expect(acks(frames)).toEqual([{ lastId: '4', results }]);
    });

    it('(c) a failed request re-queues its frames in front, and the next ack still splits per frame', async () => {
      await provider.connect();
      const frames = listen();
      mockFetch.mockRejectedValueOnce(new Error('Network error'));

      sendFrame('1');
      sendFrame('2', '3');
      await new Promise((r) => setTimeout(r, 150));
      expect(acks(frames)).toEqual([]);

      respond((ids) => ({ ack: { lastId: ids[ids.length - 1] } }));
      sendFrame('4');
      await new Promise((r) => setTimeout(r, 150));

      expect(requestOps()).toEqual([
        ['1', '2', '3'],
        ['1', '2', '3', '4'],
      ]);
      expect(acks(frames)).toEqual([{ lastId: '1' }, { lastId: '3' }, { lastId: '4' }]);
    });
  });

  it('the ops of a failed poll reach the next successful one, though the engine does not send them again', async () => {
    // After a failed poll the engine is disconnected and, since the provider's
    // recovery is only `reconnected`, not authenticated again: it sends nothing.
    // The provider's re-queue is what delivers the failed request's ops.
    const requests: string[][] = [];
    let failNextWithOps = true;
    mockFetch.mockImplementation(async (_url: string, init: { body: ArrayBuffer }) => {
      const ops = (deserialize<any>(new Uint8Array(init.body)).operations ?? []).map(
        (o: { id: string }) => String(o.id),
      );
      if (ops.length > 0) requests.push(ops);
      if (ops.length > 0 && failNextWithOps) {
        failNextWithOps = false;
        throw new Error('Network error');
      }
      return createMockResponse(defaultSyncResponse());
    });
    const storage = {
      getMeta: async (key: string) => (key === '__sys__:ormapBackfillDone' ? true : undefined),
      setMeta: jest.fn().mockResolvedValue(undefined),
      appendOpLog: jest.fn().mockResolvedValue(1),
      getPendingOps: jest.fn().mockResolvedValue([]),
      getAllKeys: jest.fn().mockResolvedValue([]),
      getAllMetaKeys: jest.fn().mockResolvedValue([]),
    } as unknown as IStorageAdapter;
    const engine = new SyncEngine({
      nodeId: 'test-node',
      connectionProvider: provider as unknown as SyncEngineConfig['connectionProvider'],
      storageAdapter: storage,
      heartbeat: { enabled: false },
    });
    try {
      // The auth-optional grace (500 ms) authenticates the engine.
      await new Promise((r) => setTimeout(r, 800));
      const timestamp = hlc.now();
      await engine.recordOperation('users', 'PUT', 'user-1', {
        record: { value: { name: 'Alice' }, timestamp },
        timestamp,
      });
      await new Promise((r) => setTimeout(r, 500));

      expect(requests).toEqual([['1'], ['1']]);
    } finally {
      engine.close();
    }
  });
});
