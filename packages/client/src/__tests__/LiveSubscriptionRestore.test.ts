/**
 * Every live subscription the client holds is registered on a newly
 * authenticated socket exactly once: standard queries, FTS hybrid queries,
 * topics, live searches and live hybrid searches. A real SyncEngine runs over
 * SingleServerProvider and a mock WebSocket; "frames" are the decoded frames
 * the mock socket received.
 */

import { serialize, deserialize } from '@topgunbuild/core';
import { SyncEngine } from '../SyncEngine';
import type { IStorageAdapter } from '../IStorageAdapter';
import { SingleServerProvider } from '../connection/SingleServerProvider';
import { SyncState } from '../SyncState';
import { SearchHandle } from '../SearchHandle';
import { HybridSearchHandle } from '../HybridSearchHandle';
import { QueryHandle } from '../QueryHandle';
import { HybridQueryHandle } from '../HybridQueryHandle';
import { TopicHandle } from '../TopicHandle';
import { logger } from '../utils/logger';

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 3;

  readyState: number = MockWebSocket.CONNECTING;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer | string }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: ((error: unknown) => void) | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- decoded msgpack frames of every type
  sentMessages: any[] = [];

  constructor(public url: string) {
    MockWebSocket.instances.push(this);
    setTimeout(() => this.open(), 0);
  }

  // Idempotent so a test can open a socket before the scheduled open fires.
  open(): void {
    if (this.readyState !== MockWebSocket.CONNECTING) return;
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.();
  }

  send(data: Uint8Array | string): void {
    if (this.readyState !== MockWebSocket.OPEN) {
      throw new Error('WebSocket is not open');
    }
    this.sentMessages.push(data instanceof Uint8Array ? deserialize(data) : JSON.parse(data));
  }

  close(): void {
    if (this.readyState === MockWebSocket.CLOSED) return;
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.({ code: 1000, reason: 'Normal closure' });
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- arbitrary server frame
  simulateMessage(message: any): void {
    this.onmessage?.({ data: new Uint8Array(serialize(message)).buffer });
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- replacing the global WebSocket constructor
(global as any).WebSocket = MockWebSocket;

function createStorage(): jest.Mocked<IStorageAdapter> {
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

const MAP = 'articles';
const SEARCH_TYPES = ['SEARCH_SUB', 'HYBRID_SEARCH_SUB'];
const SUB_TYPES = ['QUERY_SUB', 'HYBRID_QUERY_SUBSCRIBE', ...SEARCH_TYPES, 'TOPIC_SUB'];

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- decoded frames
function framesOf(ws: MockWebSocket, type: string): any[] {
  return ws.sentMessages.filter((m) => m.type === type);
}

function subFrames(ws: MockWebSocket, types: string[] = SUB_TYPES): unknown[] {
  return ws.sentMessages.filter((m) => types.includes(m.type));
}

function countByType(ws: MockWebSocket): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const type of SUB_TYPES) counts[type] = framesOf(ws, type).length;
  return counts;
}

/** Runs pending promise jobs without advancing fake time. */
async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 50; i++) {
    await Promise.resolve();
  }
}

function lastSocket(): MockWebSocket {
  return MockWebSocket.instances[MockWebSocket.instances.length - 1];
}

/** Advances fake time until a socket beyond `known` exists, then opens it. */
async function openNextSocket(known: number): Promise<MockWebSocket> {
  for (let i = 0; i < 100 && MockWebSocket.instances.length <= known; i++) {
    await jest.advanceTimersByTimeAsync(25);
  }
  expect(MockWebSocket.instances.length).toBe(known + 1);
  const ws = lastSocket();
  ws.open();
  await flushMicrotasks();
  return ws;
}

function createEngine(withToken: boolean): SyncEngine {
  const engine = new SyncEngine({
    nodeId: 'restore-node',
    connectionProvider: new SingleServerProvider({
      url: 'ws://localhost:8080',
      reconnectDelayMs: 100,
      maxReconnectDelayMs: 100,
    }),
    storageAdapter: createStorage(),
    heartbeat: { enabled: false },
  });
  if (withToken) {
    engine.setAuthToken('test-token');
  }
  return engine;
}

async function ack(ws: MockWebSocket, withToken: boolean): Promise<void> {
  ws.simulateMessage(withToken ? { type: 'AUTH_ACK' } : { type: 'DEVICE_ACK', deviceId: 'dev-1' });
  await flushMicrotasks();
}

/** Opens the engine's first socket and acknowledges it. */
async function connect(engine: SyncEngine, withToken: boolean): Promise<MockWebSocket> {
  const ws = await openNextSocket(0);
  await ack(ws, withToken);
  expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
  return ws;
}

/** Drops the current socket and opens the next one, unacknowledged. */
async function dropAndReopen(): Promise<MockWebSocket> {
  const known = MockWebSocket.instances.length;
  lastSocket().close();
  return openNextSocket(known);
}

async function reconnect(engine: SyncEngine, withToken: boolean): Promise<MockWebSocket> {
  const ws = await dropAndReopen();
  expect(subFrames(ws)).toHaveLength(0);
  await ack(ws, withToken);
  expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
  return ws;
}

function searchResp(requestId: string, keys: string[], extra: Record<string, unknown> = {}) {
  return {
    type: 'SEARCH_RESP',
    payload: {
      requestId,
      results: keys.map((key) => ({ key, value: { key }, score: 1, matchedTerms: ['alpha'] })),
      totalCount: keys.length,
      ...extra,
    },
  };
}

function hybridResp(requestId: string, keys: string[], extra: Record<string, unknown> = {}) {
  return {
    type: 'HYBRID_SEARCH_RESP',
    payload: {
      requestId,
      results: keys.map((key) => ({ key, score: 1, methodScores: { fullText: 1 } })),
      searchTimeMs: 1,
      ...extra,
    },
  };
}

function searchDelta(subscriptionId: string, key: string, changeType: string) {
  return {
    type: 'SEARCH_UPDATE',
    payload: { subscriptionId, key, value: { key }, score: 1, matchedTerms: ['alpha'], changeType },
  };
}

function hybridDelta(subscriptionId: string, key: string, changeType: string) {
  return {
    type: 'HYBRID_SEARCH_UPDATE',
    payload: { subscriptionId, key, score: 1, methodScores: { fullText: 1 }, changeType },
  };
}

/** The two live search kinds, driven through one shape. */
interface Kind {
  name: string;
  subType: string;
  create(engine: SyncEngine, query?: string): SearchHandle | HybridSearchHandle;
  resp(id: string, keys: string[], extra?: Record<string, unknown>): unknown;
  delta(id: string, key: string, changeType: string): unknown;
}

const KINDS: Kind[] = [
  {
    name: 'SearchHandle',
    subType: 'SEARCH_SUB',
    create: (engine, query = 'alpha') => new SearchHandle(engine, MAP, query),
    resp: searchResp,
    delta: searchDelta,
  },
  {
    name: 'HybridSearchHandle',
    subType: 'HYBRID_SEARCH_SUB',
    create: (engine, query = 'alpha') =>
      new HybridSearchHandle(engine, MAP, query, { methods: ['fullText'], k: 10 }),
    resp: hybridResp,
    delta: hybridDelta,
  },
];

function keysOf(handle: SearchHandle | HybridSearchHandle): string[] {
  return handle
    .getResults()
    .map((r) => r.key)
    .sort();
}

function currentId(handle: SearchHandle | HybridSearchHandle): string {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the subscription id is private by design
  return (handle as any).subscriptionId;
}

describe('Live subscription restore on reconnect', () => {
  let engine: SyncEngine | undefined;

  beforeEach(() => {
    jest.useFakeTimers();
    MockWebSocket.instances = [];
  });

  afterEach(() => {
    engine?.close();
    engine = undefined;
    jest.clearAllTimers();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  it('every reconnect restores every kind of live subscription exactly once, with new search ids', async () => {
    engine = createEngine(true);
    const first = await connect(engine, true);

    new QueryHandle(engine, MAP, {}).subscribe(() => {});
    new HybridQueryHandle(engine, MAP, {
      predicate: { op: 'match', attribute: 'title', value: 'alpha' },
    }).subscribe(() => {});
    const search = new SearchHandle(engine, MAP, 'alpha');
    const hybrid = new HybridSearchHandle(engine, MAP, 'alpha', { methods: ['fullText'] });
    new TopicHandle(engine, 'news').subscribe(() => {});

    const seenIds = new Set<string>(
      subFrames(first, SEARCH_TYPES).map(
        (m) => (m as { payload: { subscriptionId: string } }).payload.subscriptionId,
      ),
    );
    expect(seenIds.size).toBe(2);

    for (let round = 0; round < 3; round++) {
      const ws = await reconnect(engine, true);
      expect(countByType(ws)).toEqual({
        QUERY_SUB: 1,
        HYBRID_QUERY_SUBSCRIBE: 1,
        SEARCH_SUB: 1,
        HYBRID_SEARCH_SUB: 1,
        TOPIC_SUB: 1,
      });
      const ids = [
        framesOf(ws, 'SEARCH_SUB')[0].payload.subscriptionId,
        framesOf(ws, 'HYBRID_SEARCH_SUB')[0].payload.subscriptionId,
      ];
      for (const id of ids) {
        expect(seenIds.has(id)).toBe(false);
        seenIds.add(id);
      }
      expect(ids).toEqual([currentId(search), currentId(hybrid)]);
    }
  });

  it('a second acknowledgement on the same socket restores nothing', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    new QueryHandle(engine, MAP, {}).subscribe(() => {});
    new SearchHandle(engine, MAP, 'alpha');
    new HybridSearchHandle(engine, MAP, 'alpha');
    new TopicHandle(engine, 'news').subscribe(() => {});

    const ws = await reconnect(engine, true);
    const afterFirst = subFrames(ws).length;
    expect(afterFirst).toBe(4);

    await ack(ws, true);
    expect(subFrames(ws)).toHaveLength(afterFirst);
  });

  describe('token-less paths', () => {
    async function setup(): Promise<void> {
      engine = createEngine(false);
      await connect(engine, false);
      new QueryHandle(engine, MAP, {}).subscribe(() => {});
      new SearchHandle(engine, MAP, 'alpha');
      new HybridSearchHandle(engine, MAP, 'alpha');
      new TopicHandle(engine, 'news').subscribe(() => {});
    }

    const ONE_EACH = {
      QUERY_SUB: 1,
      HYBRID_QUERY_SUBSCRIBE: 0,
      SEARCH_SUB: 1,
      HYBRID_SEARCH_SUB: 1,
      TOPIC_SUB: 1,
    };

    it('(a) DEVICE_ACK restores once', async () => {
      await setup();
      const ws = await reconnect(engine!, false);
      expect(countByType(ws)).toEqual(ONE_EACH);
    });

    it('(b) the grace timer restores once and (c) a later DEVICE_ACK restores nothing more', async () => {
      await setup();
      const ws = await dropAndReopen();
      expect(engine!.getConnectionState()).toBe(SyncState.AUTHENTICATING);
      expect(subFrames(ws)).toHaveLength(0);

      await jest.advanceTimersByTimeAsync(600);
      await flushMicrotasks();
      expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
      expect(countByType(ws)).toEqual(ONE_EACH);

      await ack(ws, false);
      expect(countByType(ws)).toEqual(ONE_EACH);
    });
  });

  it('handles created before authentication subscribe once, after the ack', async () => {
    engine = createEngine(true);
    const ws = await openNextSocket(0);
    expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);
    expect(framesOf(ws, 'AUTH')).toHaveLength(1);

    const search = new SearchHandle(engine, MAP, 'alpha');
    const hybrid = new HybridSearchHandle(engine, MAP, 'alpha');
    await flushMicrotasks();
    expect(subFrames(ws, SEARCH_TYPES)).toHaveLength(0);

    await ack(ws, true);
    expect(framesOf(ws, 'SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(search),
    ]);
    expect(framesOf(ws, 'HYBRID_SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(hybrid),
    ]);
  });

  it('a handle disposed while disconnected is not restored', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    const kept = new SearchHandle(engine, MAP, 'alpha');
    const gone = new HybridSearchHandle(engine, MAP, 'alpha');
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the registry is private by design
    const registry = (engine as any).queryManager.liveSearches as Set<unknown>;
    expect(registry.size).toBe(2);

    const known = MockWebSocket.instances.length;
    lastSocket().close();
    gone.dispose();
    expect(registry.size).toBe(1);

    const ws = await openNextSocket(known);
    await ack(ws, true);
    expect(framesOf(ws, 'SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(kept),
    ]);
    expect(framesOf(ws, 'HYBRID_SEARCH_SUB')).toHaveLength(0);
  });

  describe('setQuery / setOptions / dispose before the next acknowledgement', () => {
    async function run(beforeAck: 'disconnected' | 'authenticating'): Promise<void> {
      engine = createEngine(true);
      await connect(engine, true);
      const queried = new SearchHandle(engine, MAP, 'alpha');
      const optioned = new HybridSearchHandle(engine, MAP, 'alpha', { k: 10 });
      const disposed = new SearchHandle(engine, MAP, 'gone');
      const disposedId = currentId(disposed);

      const known = MockWebSocket.instances.length;
      lastSocket().close();
      let ws: MockWebSocket | undefined;
      if (beforeAck === 'authenticating') {
        ws = await openNextSocket(known);
        expect(engine.getConnectionState()).toBe(SyncState.AUTHENTICATING);
      }

      queried.setQuery('beta');
      optioned.setOptions({ methods: ['fullText'], k: 7 });
      disposed.dispose();
      await flushMicrotasks();

      if (ws) {
        expect(subFrames(ws, SEARCH_TYPES)).toHaveLength(0);
      } else {
        expect(MockWebSocket.instances.length).toBe(known);
        ws = await openNextSocket(known);
      }
      await ack(ws, true);

      const searchSubs = framesOf(ws, 'SEARCH_SUB');
      expect(searchSubs).toHaveLength(1);
      expect(searchSubs[0].payload).toMatchObject({
        subscriptionId: currentId(queried),
        query: 'beta',
      });
      const hybridSubs = framesOf(ws, 'HYBRID_SEARCH_SUB');
      expect(hybridSubs).toHaveLength(1);
      expect(hybridSubs[0].payload).toMatchObject({
        subscriptionId: currentId(optioned),
        k: 7,
      });
      expect(searchSubs.some((m) => m.payload.subscriptionId === disposedId)).toBe(false);
    }

    it('(a) while disconnected', () => run('disconnected'));
    it('(b) on the new socket, before its acknowledgement', () => run('authenticating'));
  });

  describe.each(KINDS)('$name', (kind) => {
    async function restored(): Promise<{
      handle: SearchHandle | HybridSearchHandle;
      ws: MockWebSocket;
      listener: jest.Mock;
      previousId: string;
    }> {
      engine = createEngine(true);
      await connect(engine, true);
      const handle = kind.create(engine);
      lastSocket().simulateMessage(kind.resp(currentId(handle), ['a', 'b']));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['a', 'b']);
      const previousId = currentId(handle);

      const listener = jest.fn();
      handle.subscribe(listener);
      const ws = await reconnect(engine, true);
      expect(framesOf(ws, kind.subType)).toHaveLength(1);
      expect(currentId(handle)).not.toBe(previousId);
      listener.mockClear();
      return { handle, ws, listener, previousId };
    }

    it('the fresh result replaces the rows', async () => {
      const { handle, ws, listener } = await restored();
      // The rows of the previous socket stay visible until the response.
      expect(keysOf(handle)).toEqual(['a', 'b']);
      ws.simulateMessage(kind.resp(currentId(handle), ['b', 'c']));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['b', 'c']);
      expect(listener).toHaveBeenCalledTimes(1);
    });

    it('frames of the previous subscription are inert', async () => {
      const { handle, ws, listener, previousId } = await restored();
      ws.simulateMessage(kind.resp(previousId, ['z']));
      ws.simulateMessage(kind.delta(previousId, 'y', 'ENTER'));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['a', 'b']);
      expect(listener).not.toHaveBeenCalled();

      ws.simulateMessage(kind.resp(currentId(handle), ['z']));
      ws.simulateMessage(kind.delta(currentId(handle), 'y', 'ENTER'));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['y', 'z']);
    });

    it('a delta that arrives before the response is applied after it', async () => {
      const { handle, ws } = await restored();
      ws.simulateMessage(kind.delta(currentId(handle), 'x', 'ENTER'));
      await flushMicrotasks();
      ws.simulateMessage(kind.resp(currentId(handle), ['a', 'b']));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['a', 'b', 'x']);

      const next = await reconnect(engine!, true);
      next.simulateMessage(kind.delta(currentId(handle), 'b', 'LEAVE'));
      await flushMicrotasks();
      next.simulateMessage(kind.resp(currentId(handle), ['a', 'b']));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['a']);
    });

    it('an error response leaves the rows and notifies nobody', async () => {
      const { handle, ws, listener } = await restored();
      ws.simulateMessage(kind.resp(currentId(handle), [], { error: 'search failed' }));
      await flushMicrotasks();
      expect(keysOf(handle)).toEqual(['a', 'b']);
      expect(listener).not.toHaveBeenCalled();
    });
  });

  describe('provider listeners are attached once', () => {
    async function assertOnceOnNextSocket(start: () => void, search: SearchHandle): Promise<void> {
      const known = MockWebSocket.instances.length;
      start();
      // The socket the call created is opened before any timer is advanced.
      expect(MockWebSocket.instances.length).toBe(known + 1);
      const ws = lastSocket();
      ws.open();
      await flushMicrotasks();
      expect(framesOf(ws, 'AUTH')).toHaveLength(1);

      const spy = jest.fn();
      engine!.on('message', spy);
      ws.simulateMessage({ type: 'AUTH_REQUIRED' });
      await flushMicrotasks();
      await ack(ws, true);
      expect(engine!.getConnectionState()).toBe(SyncState.CONNECTED);
      // Any reconnect timer still pending finds the socket OPEN and returns.
      await jest.advanceTimersByTimeAsync(1000);

      expect(framesOf(ws, 'SEARCH_SUB')).toHaveLength(1);
      spy.mockClear();
      ws.simulateMessage(searchDelta(currentId(search), 'k', 'ENTER'));
      await flushMicrotasks();
      expect(spy.mock.calls.filter(([m]) => m.type === 'SEARCH_UPDATE')).toHaveLength(1);
      expect(MockWebSocket.instances.length).toBe(known + 1);
    }

    it('after setAuthToken while disconnected', async () => {
      engine = createEngine(true);
      await connect(engine, true);
      const search = new SearchHandle(engine, MAP, 'alpha');
      lastSocket().close();
      expect(engine.getConnectionState()).toBe(SyncState.DISCONNECTED);
      await assertOnceOnNextSocket(() => engine!.setAuthToken('new-token'), search);
    });

    it('after resetConnection', async () => {
      engine = createEngine(true);
      await connect(engine, true);
      const search = new SearchHandle(engine, MAP, 'alpha');
      await assertOnceOnNextSocket(() => engine!.resetConnection(), search);
    });
  });

  it('AUTH_REQUIRED after the handshake AUTH adds no AUTH, and a second AUTH_ACK no restore', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    new QueryHandle(engine, MAP, {}).subscribe(() => {});
    new SearchHandle(engine, MAP, 'alpha');
    new HybridSearchHandle(engine, MAP, 'alpha');
    new TopicHandle(engine, 'news').subscribe(() => {});

    const ws = await dropAndReopen();
    ws.simulateMessage({ type: 'AUTH_REQUIRED' });
    await flushMicrotasks();
    expect(framesOf(ws, 'AUTH')).toHaveLength(1);

    await ack(ws, true);
    await ack(ws, true);
    expect(countByType(ws)).toEqual({
      QUERY_SUB: 1,
      HYBRID_QUERY_SUBSCRIBE: 0,
      SEARCH_SUB: 1,
      HYBRID_SEARCH_SUB: 1,
      TOPIC_SUB: 1,
    });
  });

  it('one handle that throws does not stop the restore', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    const failing = new SearchHandle(engine, MAP, 'alpha');
    const healthy = new SearchHandle(engine, MAP, 'beta');
    new TopicHandle(engine, 'news').subscribe(() => {});

    const boom = new Error('resubscribe failed');
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- spying on the restore entry point of one instance
    jest.spyOn(failing as any, 'resubscribe').mockImplementation(() => {
      throw boom;
    });
    const errorSpy = jest.spyOn(logger, 'error');

    const ws = await dropAndReopen();
    await ack(ws, true);

    expect(framesOf(ws, 'SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(healthy),
    ]);
    expect(framesOf(ws, 'TOPIC_SUB')).toHaveLength(1);
    expect(
      errorSpy.mock.calls.filter(([arg]) => (arg as { err?: unknown })?.err === boom),
    ).toHaveLength(1);
    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
  });

  it('a query whose subscribe throws does not stop the live search restore', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    new QueryHandle(engine, MAP, {}).subscribe(() => {});
    const search = new SearchHandle(engine, MAP, 'alpha');

    const boom = new Error('query subscribe failed');
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the query restore step is private
    jest.spyOn((engine as any).queryManager, 'sendQuerySubscription').mockImplementation(() => {
      throw boom;
    });
    const errorSpy = jest.spyOn(logger, 'error');

    const ws = await dropAndReopen();
    await ack(ws, true);

    expect(framesOf(ws, 'SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(search),
    ]);
    expect(
      errorSpy.mock.calls.filter(([arg]) => (arg as { err?: unknown })?.err === boom),
    ).toHaveLength(1);
    expect(engine.getConnectionState()).toBe(SyncState.CONNECTED);
  });

  it('a live search whose first subscribe throws stays registered and is restored', async () => {
    engine = createEngine(true);
    await connect(engine, true);
    const boom = new Error('send failed');
    jest.spyOn(engine, 'send').mockImplementationOnce(() => {
      throw boom;
    });
    const errorSpy = jest.spyOn(logger, 'error');

    let search: SearchHandle | undefined;
    expect(() => {
      search = new SearchHandle(engine!, MAP, 'alpha');
    }).not.toThrow();
    expect(
      errorSpy.mock.calls.filter(([arg]) => (arg as { err?: unknown })?.err === boom),
    ).toHaveLength(1);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the registry is private by design
    const registry = (engine as any).queryManager.liveSearches as Set<unknown>;
    expect(registry.has(search)).toBe(true);

    const ws = await dropAndReopen();
    await ack(ws, true);
    expect(framesOf(ws, 'SEARCH_SUB').map((m) => m.payload.subscriptionId)).toEqual([
      currentId(search!),
    ]);

    search!.dispose();
    expect(registry.has(search)).toBe(false);
  });
});
