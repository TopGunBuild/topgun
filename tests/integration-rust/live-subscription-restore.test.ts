/**
 * Live subscriptions survive a reconnect against the real Rust server.
 *
 * The server drops every subscription of a connection when its socket closes,
 * so the client has to subscribe each live query, live search, live hybrid
 * search and topic again on the next socket, and the server's fresh result has
 * to replace what the handles showed. The subscriber reaches the server through
 * an in-test TCP proxy, so a network drop can be made without restarting the
 * server: cutting the proxy closes the socket on both ends, the client takes
 * its production `disconnected` path and the server releases the connection's
 * subscriptions. The writer connects to the server directly.
 */

import * as net from 'net';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { HybridSearchHandle, QueryHandle, SearchHandle } from '@topgunbuild/client';

import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  waitUntil,
  MemoryStorageAdapter,
  SpawnedServer,
  TestClient,
} from './helpers';

const MAP = 'restore-articles';
const TOPIC = 'restore-news';
const QUERY = 'alpha';
const EMBED_DIM = 64;
const SERVER_ENV = {
  TOPGUN_EMBEDDING_PROVIDER: 'deterministic',
  TOPGUN_EMBEDDING_DIMENSION: String(EMBED_DIM),
  TOPGUN_VECTOR_MAPS: JSON.stringify({ [MAP]: { fields: ['title'], dimension: EMBED_DIM } }),
};
const WAIT_MS = 20_000;

/** Reserve an ephemeral port, then release it so the server can bind it later. */
function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      if (addr && typeof addr === 'object') {
        const { port } = addr;
        srv.close(() => resolve(port));
      } else {
        srv.close(() => reject(new Error('Could not determine free port')));
      }
    });
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Pipes every accepted socket to the server port. `cut()` destroys every piped
 * pair and refuses new connections until `heal()`. A failed upstream connect
 * (the server is down) destroys the accepted socket at once, so the client sees
 * a closed socket and backs off instead of waiting for its connection timeout.
 */
class CutProxy {
  private server: net.Server | null = null;
  private sockets = new Set<net.Socket>();
  private accepting = true;
  port = 0;

  constructor(private readonly upstreamPort: number) {}

  async start(): Promise<void> {
    this.server = net.createServer((downstream) => {
      if (!this.accepting) {
        downstream.on('error', () => {});
        downstream.destroy();
        return;
      }
      const upstream = net.connect(this.upstreamPort, '127.0.0.1');
      const destroyPair = (): void => {
        downstream.destroy();
        upstream.destroy();
        this.sockets.delete(downstream);
        this.sockets.delete(upstream);
      };
      for (const socket of [downstream, upstream]) {
        this.sockets.add(socket);
        socket.on('error', destroyPair);
        socket.on('close', destroyPair);
      }
      downstream.pipe(upstream);
      upstream.pipe(downstream);
    });
    await new Promise<void>((resolve) => this.server!.listen(0, '127.0.0.1', resolve));
    this.port = (this.server.address() as net.AddressInfo).port;
  }

  cut(): void {
    this.accepting = false;
    for (const socket of this.sockets) socket.destroy();
    this.sockets.clear();
  }

  heal(): void {
    this.accepting = true;
  }

  async close(): Promise<void> {
    this.cut();
    await new Promise<void>((resolve) =>
      this.server ? this.server.close(() => resolve()) : resolve(),
    );
  }
}

function makeClient(port: number, withToken: boolean, user: string): TopGunClient {
  const token = createTestToken(user, ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://127.0.0.1:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    ...(withToken ? { auth: { getToken: async () => token } } : {}),
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

async function waitForConnected(client: TopGunClient): Promise<void> {
  await waitUntil(() => client.getConnectionState() === SyncState.CONNECTED, WAIT_MS, 50);
}

function sortedKeys(keys: Iterable<string>): string[] {
  return [...keys].sort();
}

function subscriptionId(handle: SearchHandle<unknown> | HybridSearchHandle<unknown>): string {
  // The subscription id is private by design; it rotates on every restore.
  return (handle as unknown as { subscriptionId: string }).subscriptionId;
}

/** The subscriber with its four handles and what it observed. */
class Subscriber {
  readonly client: TopGunClient;
  query!: QueryHandle<unknown>;
  search!: SearchHandle<unknown>;
  hybrid!: HybridSearchHandle<unknown>;
  queryKeys: string[] = [];
  topicData: Array<Record<string, unknown>> = [];
  /** Every frame the engine emitted, in order. */
  frames: Array<{ type: string; payload?: Record<string, unknown> }> = [];
  /** Ids answered on the current socket (cleared on every disconnect). */
  answeredIds = new Set<string>();
  /** Per CONNECTED transition: whether that socket saw a DEVICE_ACK first. */
  connectedAfterDeviceAck: boolean[] = [];
  private deviceAckOnSocket = false;

  constructor(port: number, withToken: boolean) {
    this.client = makeClient(port, withToken, 'restore-subscriber');
    type Frame = { type: string; payload?: Record<string, unknown> };
    // The engine is private to the client; its message bus is what a handle sees.
    const engine = (
      this.client as unknown as {
        syncEngine: { on(event: 'message', handler: (message: unknown) => void): void };
      }
    ).syncEngine;
    engine.on('message', (raw) => {
      const message = raw as Frame;
      this.frames.push(message);
      if (message.type === 'QUERY_RESP') {
        this.answeredIds.add(String(message.payload?.queryId));
      } else if (message.type === 'SEARCH_RESP' || message.type === 'HYBRID_SEARCH_RESP') {
        this.answeredIds.add(String(message.payload?.requestId));
      } else if (message.type === 'DEVICE_ACK') {
        this.deviceAckOnSocket = true;
      }
    });
    this.client.onConnectionStateChange((event) => {
      if (event.to === SyncState.DISCONNECTED) {
        this.answeredIds.clear();
        this.deviceAckOnSocket = false;
      } else if (event.to === SyncState.CONNECTED) {
        this.connectedAfterDeviceAck.push(this.deviceAckOnSocket);
      }
    });
  }

  /** Creates the handles; each is asserted to exist before the first restore. */
  createHandles(): void {
    const notAuthenticated = (): void => {
      expect([SyncState.SYNCING, SyncState.CONNECTED]).not.toContain(
        this.client.getConnectionState(),
      );
    };
    notAuthenticated();
    this.query = this.client.query(MAP, {});
    this.query.subscribe((rows) => {
      this.queryKeys = sortedKeys(rows.map((row) => (row as { _key: string })._key));
    });
    notAuthenticated();
    this.search = this.client.searchSubscribe(MAP, QUERY);
    notAuthenticated();
    this.hybrid = this.client.hybridSearchSubscribe(MAP, QUERY, { methods: ['fullText'], k: 50 });
    notAuthenticated();
    this.client.topic(TOPIC).subscribe((data) => {
      this.topicData.push(data as Record<string, unknown>);
    });
  }

  keySets(): Record<string, string[]> {
    return {
      query: this.queryKeys,
      search: sortedKeys(this.search.getResults().map((r) => r.key)),
      hybrid: sortedKeys(this.hybrid.getResults().map((r) => r.key)),
    };
  }

  /**
   * Waits until each handle's current subscription was answered on the current
   * socket. The server evaluates a subscribe's snapshot before it registers the
   * subscription, so a write indexed in between would be in neither.
   */
  async waitForAnswers(): Promise<void> {
    await waitUntil(
      () =>
        this.client.getConnectionState() === SyncState.CONNECTED &&
        this.answeredIds.has(this.query.id) &&
        this.answeredIds.has(subscriptionId(this.search)) &&
        this.answeredIds.has(subscriptionId(this.hybrid)),
      WAIT_MS,
      50,
    );
  }

  async waitForKeys(expected: string[]): Promise<void> {
    const want = { query: expected, search: expected, hybrid: expected };
    try {
      await waitUntil(() => JSON.stringify(this.keySets()) === JSON.stringify(want), WAIT_MS, 50);
    } finally {
      expect(this.keySets()).toEqual(want);
    }
  }

  /**
   * How often each handle was told that `key` entered its results, over every
   * subscription id: a second registration (under any id) would deliver a
   * second ENTER. The server may follow the ENTER with an UPDATE of the same
   * key when it indexes the record again, so UPDATEs are not counted; every
   * frame for `key` must still carry the handle's current id.
   */
  entersFor(key: string): Record<string, number> {
    const count = (type: string, idField: string, id: string): number => {
      const frames = this.frames.filter((f) => f.type === type && f.payload?.key === key);
      expect(frames.map((f) => f.payload?.[idField])).toEqual(frames.map(() => id));
      return frames.filter((f) => f.payload?.changeType === 'ENTER').length;
    };
    return {
      query: count('QUERY_UPDATE', 'queryId', this.query.id),
      search: count('SEARCH_UPDATE', 'subscriptionId', subscriptionId(this.search)),
      hybrid: count('HYBRID_SEARCH_UPDATE', 'subscriptionId', subscriptionId(this.hybrid)),
    };
  }

  /**
   * A topic subscription has no response frame, so a publish right after the
   * subscribe can race it. Probe messages are published until one arrives; then
   * the test message is published once and must arrive exactly once.
   */
  async expectTopicMessageOnce(writer: TopGunClient, tag: string): Promise<void> {
    let probe = 0;
    await waitUntil(
      () => {
        writer.topic(TOPIC).publish({ probe: `${tag}-${probe++}` });
        return this.topicData.some((d) => String(d.probe ?? '').startsWith(`${tag}-`));
      },
      WAIT_MS,
      200,
    );
    writer.topic(TOPIC).publish({ test: tag });
    await waitUntil(() => this.topicData.some((d) => d.test === tag), WAIT_MS, 50);
    await sleep(500);
    expect(this.topicData.filter((d) => d.test === tag)).toHaveLength(1);
  }
}

async function put(writer: TopGunClient, key: string): Promise<void> {
  writer.getMap<string, { title: string }>(MAP).set(key, { title: `${QUERY} ${key}` });
  await waitUntil(() => writer.getPendingOpsCount() === 0, WAIT_MS, 25);
}

async function remove(writer: TopGunClient, key: string): Promise<void> {
  writer.getMap<string, { title: string }>(MAP).remove(key);
  await waitUntil(() => writer.getPendingOpsCount() === 0, WAIT_MS, 25);
}

/** The key sets a fresh one-shot QUERY, SEARCH and HYBRID_SEARCH return on a new raw connection. */
async function freshKeySets(port: number): Promise<Record<string, string[]>> {
  const raw: TestClient = await createRustTestClient(port, {
    nodeId: 'restore-fresh',
    userId: 'restore-fresh',
    roles: ['ADMIN'],
  });
  try {
    await raw.waitForMessage('AUTH_ACK', WAIT_MS);
    raw.messages.length = 0;
    raw.send({ type: 'QUERY_SUB', payload: { queryId: 'fresh-q', mapName: MAP, query: {} } });
    const queryResp = await raw.waitForMessage('QUERY_RESP', WAIT_MS);
    raw.send({ type: 'QUERY_UNSUB', payload: { queryId: 'fresh-q' } });
    raw.messages.length = 0;
    raw.send({ type: 'SEARCH', payload: { requestId: 'fresh-s', mapName: MAP, query: QUERY } });
    const searchResp = await raw.waitForMessage('SEARCH_RESP', WAIT_MS);
    raw.messages.length = 0;
    raw.send({
      type: 'HYBRID_SEARCH',
      payload: {
        requestId: 'fresh-h',
        mapName: MAP,
        queryText: QUERY,
        methods: ['fullText'],
        k: 50,
      },
    });
    const hybridResp = await raw.waitForMessage('HYBRID_SEARCH_RESP', WAIT_MS);
    const keys = (resp: { payload: { results: Array<{ key: string }> } }): string[] =>
      sortedKeys(resp.payload.results.map((r) => r.key));
    return { query: keys(queryResp), search: keys(searchResp), hybrid: keys(hybridResp) };
  } finally {
    raw.close();
  }
}

describe('Integration: live subscriptions are restored on every reconnect', () => {
  let server: SpawnedServer | null = null;
  let proxy: CutProxy | null = null;
  let subscriber: Subscriber | null = null;
  let writer: TopGunClient | null = null;

  afterEach(async () => {
    await subscriber?.client.close().catch(() => {});
    await writer?.close().catch(() => {});
    await proxy?.close().catch(() => {});
    await server?.cleanup().catch(() => {});
    subscriber = null;
    writer = null;
    proxy = null;
    server = null;
  });

  async function setUp(
    withToken: boolean,
  ): Promise<{ serverPort: number; env: Record<string, string> }> {
    const serverPort = await findFreePort();
    const env: Record<string, string> = withToken
      ? { ...SERVER_ENV }
      : { ...SERVER_ENV, TOPGUN_NO_AUTH: 'true' };
    server = await spawnRustServer({ port: serverPort, env });
    proxy = new CutProxy(serverPort);
    await proxy.start();

    subscriber = new Subscriber(proxy.port, withToken);
    await subscriber.client.start();
    subscriber.createHandles();

    writer = makeClient(serverPort, withToken, 'restore-writer');
    await writer.start();
    await waitForConnected(writer);
    return { serverPort, env };
  }

  async function baseline(sub: Subscriber, w: TopGunClient): Promise<void> {
    await sub.waitForAnswers();
    await put(w, 'a');
    await put(w, 'b');
    await sub.waitForKeys(['a', 'b']);
    await sub.expectTopicMessageOnce(w, 'baseline');
  }

  async function networkDrop(sub: Subscriber, w: TopGunClient, p: CutProxy): Promise<void> {
    p.cut();
    await waitUntil(() => sub.client.getConnectionState() !== SyncState.CONNECTED, WAIT_MS, 25);
    await remove(w, 'a');
    await put(w, 'c');
    p.heal();
    await sub.waitForAnswers();
    await sub.waitForKeys(['b', 'c']);

    await put(w, 'd');
    await sub.waitForKeys(['b', 'c', 'd']);
    await sleep(500);
    expect(sub.entersFor('d')).toEqual({ query: 1, search: 1, hybrid: 1 });
    await sub.expectTopicMessageOnce(w, 'after-drop');
  }

  test('with a token: baseline, network drop and server restart', async () => {
    const { serverPort, env } = await setUp(true);
    const sub = subscriber!;

    await baseline(sub, writer!);
    await networkDrop(sub, writer!, proxy!);

    // Server restart: the proxy is untouched, the server comes back empty.
    await writer!.close();
    writer = null;
    await server!.cleanup();
    server = null;
    server = await spawnRustServer({ port: serverPort, env });
    writer = makeClient(serverPort, true, 'restore-writer');
    await writer.start();
    await waitForConnected(writer);

    await sub.waitForAnswers();
    await put(writer, 'e');

    let fresh: Record<string, string[]> = {};
    await waitUntil(
      async () => {
        fresh = await freshKeySets(serverPort);
        return Object.values(fresh).every((keys) => keys.includes('e'));
      },
      WAIT_MS,
      200,
    );
    // The store is null: anything but `e` on the server means the subscriber
    // pushed records back.
    for (const keys of Object.values(fresh)) {
      expect(keys).not.toContain('b');
    }
    await sub.waitForKeys(fresh.query);
    expect(sub.keySets()).toEqual(fresh);
  }, 120_000);

  test('token-less subscriber (DEVICE_ACK path): baseline and network drop', async () => {
    await setUp(false);
    const sub = subscriber!;

    await baseline(sub, writer!);
    await networkDrop(sub, writer!, proxy!);

    expect(sub.connectedAfterDeviceAck.length).toBeGreaterThanOrEqual(2);
    expect(sub.connectedAfterDeviceAck.every(Boolean)).toBe(true);
  }, 120_000);
});
