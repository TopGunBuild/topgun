/**
 * Integration: a server that has lost an acknowledged OR-Map remove gets it
 * back from the client that issued it.
 *
 * With the default `batched` WAL fsync policy a remove is acknowledged before
 * it is durable, so an unclean stop inside the group-commit window leaves a
 * server that still holds the value while the client that removed it holds an
 * acknowledged tombstone. The client suppresses the value locally either way;
 * what has to be true is that its next sync hands the tombstone back, so the
 * value is removed on the server (and therefore for every other client) too.
 *
 * A kill inside a ~10 ms window cannot be hit deterministically, so the test
 * produces the same server state by other means: the server is stopped, its
 * data directory (store and WAL) is copied, the remove is then issued against
 * a server restarted on the original directory, and finally a server is started
 * on the COPY. That server holds exactly what a crashed one would hold: every
 * write up to the snapshot, and nothing of the acknowledged remove.
 *
 * The outcome is asserted on server state, read through a fresh connection's
 * Merkle walk. ORMAP_PUSH_DIFF has no response, so the read is repeated until
 * the value is gone or a deadline passes; a run that reaches the deadline fails
 * on the last observed server state.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncEngine, SyncState } from '@topgunbuild/client';

import { ORMapSyncHandler } from '../../packages/client/src/sync/ORMapSyncHandler';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  waitUntil,
  MemoryStorageAdapter,
  SpawnedServer,
} from './helpers';

const MAP_NAME = 'lost_remove_heal_or';
/** Holds the removed value and one that stays, so the client still has a live record for it. */
const KEY_KEEPING_A_VALUE = 'key-keeping-a-value';
/** Holds only the removed value, so the client has no live record left for it. */
const KEY_EMPTIED = 'key-emptied';
const REMOVED = 'removed-value';
const KEPT = 'kept-value';
const KEYS = [KEY_KEEPING_A_VALUE, KEY_EMPTIED];

/** Upper bound on any single setup wait. It only ever turns a hang into a failure. */
const SIGNAL_TIMEOUT_MS = 20_000;
/** How long the server is given to apply the pushed tombstones after the walk. */
const CONVERGENCE_DEADLINE_MS = 15_000;

interface RootPayload {
  mapName: string;
  rootHash: number;
  fullResync?: boolean;
}

interface LeafPayload {
  mapName: string;
  entries: Array<{ key: string; records: unknown[]; tombstones: string[] }>;
}

interface LeafInvocation {
  payload: LeafPayload;
  /** Settles when the handler has merged the leaf and sent its push. */
  done: Promise<void>;
}

/** What the server holds for one key: its live records and its tombstone tags. */
interface ServerKeyState {
  values: string[];
  tags: string[];
  tombstones: string[];
}

type ServerState = Map<string, ServerKeyState>;

/**
 * Records events as they happen and lets a caller wait for the first one that
 * matches. The deadline only bounds a run in which the event never comes, and
 * that case is reported as `null`.
 */
class Captured<T> {
  readonly items: T[] = [];
  private waiters: Array<{ matches: (item: T) => boolean; resolve: (item: T) => void }> = [];

  record(item: T): void {
    this.items.push(item);
    const ready = this.waiters.filter((w) => w.matches(item));
    this.waiters = this.waiters.filter((w) => !w.matches(item));
    ready.forEach((w) => w.resolve(item));
  }

  /** Forgets everything recorded so far, so a later wait only sees later events. */
  reset(): void {
    this.items.length = 0;
  }

  first(matches: (item: T) => boolean, timeoutMs: number): Promise<T | null> {
    const existing = this.items.find(matches);
    if (existing !== undefined) return Promise.resolve(existing);
    return new Promise((resolve) => {
      const waiter = {
        matches,
        resolve: (item: T) => {
          clearTimeout(timer);
          resolve(item);
        },
      };
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        resolve(null);
      }, timeoutMs);
      this.waiters.push(waiter);
    });
  }
}

function engineOf(client: TopGunClient): SyncEngine {
  return (client as unknown as { syncEngine: SyncEngine }).syncEngine;
}

const sorted = (items: Iterable<string>): string[] => Array.from(items).sort();

describe('Integration: a server that lost an acknowledged OR-Map remove', () => {
  let server: SpawnedServer | null = null;
  let client: TopGunClient | null = null;
  let dataDir: string | null = null;
  let snapshotDir: string | null = null;

  const roots = new Captured<RootPayload>();
  const leaves = new Captured<LeafInvocation>();
  let recordedOps: jest.SpyInstance;
  let observerCount = 0;

  // --- What the scenario observed; the tests below only read these. ---
  let reconnectRoot: RootPayload | null = null;
  let walkReachedBothKeys = false;
  /** Every value the reconnected client showed a subscriber, per key. */
  const renderedAfterReconnect = new Map<string, string[]>();
  let clientValuesAfterReconnect = new Map<string, string[]>();
  let serverAfterHeal: ServerState = new Map();

  /**
   * Starts a server on `dir`.
   *
   * `per_op` makes every acknowledged write durable in the WAL at once, so what
   * a directory holds when its server stops is exactly what was acknowledged:
   * the snapshot deterministically contains the adds, and the loss of the
   * remove comes from the snapshot alone, never from an fsync race.
   *
   * The write-behind flush interval is set far above the test's duration so the
   * durability watermark stays 0 and tombstone protection never activates. An
   * active gate would route the reconnect to a full-resync REPLACE, which makes
   * the client adopt the server's state wholesale and is a different scenario
   * from the incremental walk under test; the fullResync assertion below fails
   * if that ever happens.
   */
  function startServer(dir: string): Promise<SpawnedServer> {
    return spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_WAL_FSYNC_POLICY: 'per_op',
        TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000',
        TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '600000',
      },
    });
  }

  /** Stops the server and returns only once its process is gone and the store is unlocked. */
  async function stopServer(): Promise<void> {
    if (!server) return;
    const stopping = server;
    server = null;
    await stopping.cleanup();
    await waitUntil(
      () => stopping.process.exitCode !== null || stopping.process.signalCode !== null,
      SIGNAL_TIMEOUT_MS,
    );
  }

  function makeClient(port: number, storage: MemoryStorageAdapter): TopGunClient {
    const token = createTestToken('lost-remove-user', ['ADMIN']);
    return new TopGunClient({
      serverUrl: `ws://localhost:${port}/ws`,
      storage,
      auth: { getToken: async () => token },
      backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
    });
  }

  /** Waits until every write recorded so far has been acknowledged by the server. */
  async function expectAllWritesAcked(c: TopGunClient): Promise<void> {
    const opIds: string[] = await Promise.all(recordedOps.mock.results.map((r) => r.value));
    for (const opId of opIds) {
      expect(await engineOf(c).waitForOpSynced(opId, SIGNAL_TIMEOUT_MS)).toBe('synced');
    }
  }

  /**
   * Waits until this connection's Merkle walk has delivered, merged and pushed
   * back the leaf of every key under test.
   */
  async function walkSettledForBothKeys(): Promise<boolean> {
    for (const key of KEYS) {
      const leaf = await leaves.first(
        (l) => l.payload.mapName === MAP_NAME && l.payload.entries.some((e) => e.key === key),
        SIGNAL_TIMEOUT_MS,
      );
      if (!leaf) return false;
      await leaf.done;
    }
    return true;
  }

  /**
   * Reads what the server holds for the map, over a connection of its own: a
   * full Merkle walk from an empty local tree, which makes the server hand over
   * every key's records and tombstones.
   */
  async function readServer(port: number): Promise<ServerState> {
    const observer = await createRustTestClient(port, {
      nodeId: `lost-remove-observer-${observerCount++}`,
      userId: 'lost-remove-observer',
      roles: ['ADMIN'],
    });
    try {
      await observer.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
      observer.send({
        type: 'ORMAP_SYNC_INIT',
        mapName: MAP_NAME,
        rootHash: 0,
        bucketHashes: {},
        lastSyncTimestamp: 0,
      });
      const root = await observer.waitForMessage('ORMAP_SYNC_RESP_ROOT', SIGNAL_TIMEOUT_MS);

      const state: ServerState = new Map();
      if (Number(root.payload.rootHash) === 0) return state;

      // A request for a path the server holds nothing under gets no answer, so
      // only children reported with a non-zero hash are followed.
      const pendingPaths = [''];
      let consumed = 0;
      while (pendingPaths.length > 0) {
        const requested = pendingPaths.shift() as string;
        observer.send({
          type: 'ORMAP_MERKLE_REQ_BUCKET',
          payload: { mapName: MAP_NAME, path: requested },
        });

        let answer: { type: string; payload: Record<string, unknown> } | undefined;
        await waitUntil(
          () => {
            for (; consumed < observer.messages.length; consumed++) {
              const msg = observer.messages[consumed];
              const isAnswer =
                (msg.type === 'ORMAP_SYNC_RESP_BUCKETS' || msg.type === 'ORMAP_SYNC_RESP_LEAF') &&
                msg.payload?.mapName === MAP_NAME &&
                msg.payload?.path === requested;
              if (isAnswer) {
                answer = msg;
                consumed++;
                return true;
              }
            }
            return false;
          },
          SIGNAL_TIMEOUT_MS,
          20,
        );
        if (!answer) throw new Error(`no answer for Merkle path "${requested}"`);

        if (answer.type === 'ORMAP_SYNC_RESP_BUCKETS') {
          const buckets = answer.payload.buckets as Record<string, number>;
          for (const [child, hash] of Object.entries(buckets)) {
            if (Number(hash) !== 0) pendingPaths.push(requested + child);
          }
        } else {
          const entries = answer.payload.entries as Array<{
            key: string;
            records: Array<{ value: string; tag: string }>;
            tombstones: string[];
          }>;
          for (const entry of entries) {
            state.set(entry.key, {
              values: sorted(entry.records.map((r) => r.value)),
              tags: sorted(entry.records.map((r) => r.tag)),
              tombstones: sorted(entry.tombstones),
            });
          }
        }
      }
      return state;
    } finally {
      observer.close();
    }
  }

  /**
   * Reads the server until `settled` holds or the deadline passes, and returns
   * the last state read either way: the caller asserts on it, so a deadline
   * shows up as a failed expectation on real server state. A read that itself
   * fails is not retried and rejects.
   */
  async function readServerUntil(
    port: number,
    settled: (state: ServerState) => boolean,
    deadlineMs: number,
  ): Promise<ServerState> {
    const deadline = Date.now() + deadlineMs;
    for (;;) {
      const state = await readServer(port);
      if (settled(state) || Date.now() >= deadline) return state;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  const valuesOn = (state: ServerState, key: string): string[] => state.get(key)?.values ?? [];

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-lost-remove-'));
    snapshotDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-lost-remove-snapshot-'));

    const originalRoot = ORMapSyncHandler.prototype.handleORMapSyncRespRoot;
    jest.spyOn(ORMapSyncHandler.prototype, 'handleORMapSyncRespRoot').mockImplementation(function (
      this: ORMapSyncHandler,
      payload,
    ) {
      roots.record(payload);
      return originalRoot.call(this, payload);
    });

    const originalLeaf = ORMapSyncHandler.prototype.handleORMapSyncRespLeaf;
    jest.spyOn(ORMapSyncHandler.prototype, 'handleORMapSyncRespLeaf').mockImplementation(function (
      this: ORMapSyncHandler,
      payload,
    ) {
      const done = originalLeaf.call(this, payload);
      leaves.record({ payload, done });
      return done;
    });

    recordedOps = jest.spyOn(SyncEngine.prototype, 'recordOperation');

    const storage = new MemoryStorageAdapter();
    const connect = async (port: number): Promise<TopGunClient> => {
      roots.reset();
      leaves.reset();
      const connected = makeClient(port, storage);
      client = connected;
      await connected.start();
      await waitUntil(
        () => connected.getConnectionState() === SyncState.CONNECTED,
        SIGNAL_TIMEOUT_MS,
      );
      return connected;
    };
    const disconnect = async (): Promise<void> => {
      if (client) await client.close();
      client = null;
    };

    // --- 1. The client adds its values and every add is acknowledged. ---
    server = await startServer(dataDir);
    let current = await connect(server.port);
    let map = current.getORMap<string, string>(MAP_NAME);
    // One add at a time: an add still waiting for its acknowledgement is sent
    // again with the next one, and the server stores every delivery under a
    // fresh tag, which would only add duplicate records to the scenario.
    map.add(KEY_KEEPING_A_VALUE, REMOVED);
    await expectAllWritesAcked(current);
    map.add(KEY_KEEPING_A_VALUE, KEPT);
    await expectAllWritesAcked(current);
    map.add(KEY_EMPTIED, REMOVED);
    await expectAllWritesAcked(current);
    await disconnect();

    // --- 2. The client learns the tags the server stored the values under. ---
    // The server stamps every added record with a tag of its own, so the tag
    // the client created locally is not the one the server holds. A remove
    // names tags; one issued now would name tags the server has never seen and
    // remove nothing there. A sync walk brings the server's records to the
    // client and the client's to the server, after which both name the same
    // tags and a remove means the same thing on both sides.
    current = await connect(server.port);
    map = current.getORMap<string, string>(MAP_NAME);
    expect(await walkSettledForBothKeys()).toBe(true);

    const clientTags = (key: string): string[] => sorted(map.getRecordsMap(key)?.keys() ?? []);
    // The client's half of the exchange is a push with no response, so the
    // server is read until it holds the client's tags or the deadline passes.
    const beforeSnapshot = await readServerUntil(
      server.port,
      (state) =>
        KEYS.every(
          (key) => JSON.stringify(state.get(key)?.tags ?? []) === JSON.stringify(clientTags(key)),
        ),
      SIGNAL_TIMEOUT_MS,
    );
    // Precondition: the client holds every record under the tag the server
    // holds it under, so the removes below name the server's tags.
    for (const key of KEYS) {
      expect({ key, serverTags: beforeSnapshot.get(key)?.tags }).toEqual({
        key,
        serverTags: clientTags(key),
      });
      expect(beforeSnapshot.get(key)?.tombstones).toEqual([]);
    }
    await disconnect();

    // --- 3. Snapshot the server's durable state as of "before the remove". ---
    await stopServer();
    fs.cpSync(dataDir, snapshotDir, { recursive: true });

    // --- 4. The client removes the value; the server acknowledges the remove. ---
    server = await startServer(dataDir);
    current = await connect(server.port);
    map = current.getORMap<string, string>(MAP_NAME);
    // Step 2 left the client and the server holding the same tags, so this
    // connection has nothing to exchange and no leaf to wait for: its sync ends
    // at equal roots, or at equal buckets when the root was compared before the
    // map had loaded from storage. The removes need the loaded map, so the wait
    // is on the values the client persisted.
    const restored = (): boolean =>
      map.get(KEY_KEEPING_A_VALUE).includes(REMOVED) &&
      map.get(KEY_KEEPING_A_VALUE).includes(KEPT) &&
      map.get(KEY_EMPTIED).includes(REMOVED);
    await waitUntil(restored, SIGNAL_TIMEOUT_MS);
    expect(restored()).toBe(true);

    recordedOps.mockClear();
    expect(map.remove(KEY_KEEPING_A_VALUE, REMOVED).length).toBeGreaterThanOrEqual(1);
    expect(map.remove(KEY_EMPTIED, REMOVED).length).toBeGreaterThanOrEqual(1);
    expect(recordedOps.mock.calls.length).toBeGreaterThanOrEqual(2);
    await expectAllWritesAcked(current);

    // Precondition: the remove really took effect on the server.
    const afterRemove = await readServer(server.port);
    expect(valuesOn(afterRemove, KEY_KEEPING_A_VALUE)).not.toContain(REMOVED);
    expect(valuesOn(afterRemove, KEY_KEEPING_A_VALUE)).toContain(KEPT);
    expect(valuesOn(afterRemove, KEY_EMPTIED)).toEqual([]);
    await disconnect();

    // --- 5. The server comes back without the acknowledged remove. ---
    await stopServer();
    server = await startServer(snapshotDir);

    // Precondition: this server serves the removed value as live again and
    // knows no tombstone for it.
    const afterLoss = await readServer(server.port);
    for (const key of KEYS) {
      expect(valuesOn(afterLoss, key)).toContain(REMOVED);
      expect(afterLoss.get(key)?.tombstones).toEqual([]);
    }

    // --- 6. The client reconnects with its persisted state and syncs. ---
    roots.reset();
    leaves.reset();
    const reconnected = makeClient(server.port, storage);
    client = reconnected;
    const reconnectedMap = reconnected.getORMap<string, string>(MAP_NAME);
    reconnectedMap.subscribe((entries) => {
      for (const [key, values] of entries) {
        renderedAfterReconnect.set(key, [...(renderedAfterReconnect.get(key) ?? []), ...values]);
      }
    });
    await reconnected.start();

    reconnectRoot = await roots.first((root) => root.mapName === MAP_NAME, SIGNAL_TIMEOUT_MS);
    walkReachedBothKeys = await walkSettledForBothKeys();

    // --- 7. Give the server time to apply what the walk pushed back. ---
    // If the deadline passes, the tests below fail on the state last observed.
    serverAfterHeal = await readServerUntil(
      server.port,
      (state) => KEYS.every((key) => !valuesOn(state, key).includes(REMOVED)),
      CONVERGENCE_DEADLINE_MS,
    );

    clientValuesAfterReconnect = new Map(KEYS.map((key) => [key, reconnectedMap.get(key)]));
  }, 180_000);

  afterAll(async () => {
    jest.restoreAllMocks();
    if (client) await client.close().catch(() => {});
    if (server) await server.cleanup().catch(() => {});
    // Removed only after the server has stopped: it holds the store and the WAL open.
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    if (snapshotDir) fs.rmSync(snapshotDir, { recursive: true, force: true });
  });

  /**
   * Everything the convergence assertions depend on. Each of them runs this
   * first, so none can pass on a run that healed through a full-resync REPLACE
   * or whose walk never reached the keys.
   */
  function expectIncrementalWalkReachedBothKeys(): void {
    expect(reconnectRoot).not.toBeNull();
    expect(
      reconnectRoot?.fullResync
        ? 'reconnect was routed to full resync — the incremental walk was not exercised'
        : 'incremental sync',
    ).toBe('incremental sync');
    expect(
      walkReachedBothKeys ? 'walk reached both keys' : 'the walk did not deliver both keys',
    ).toBe('walk reached both keys');
  }

  test('the client never shows the removed value after it reconnects', () => {
    expectIncrementalWalkReachedBothKeys();

    for (const key of KEYS) {
      expect(renderedAfterReconnect.get(key) ?? []).not.toContain(REMOVED);
      expect(clientValuesAfterReconnect.get(key)).not.toContain(REMOVED);
    }
    expect(clientValuesAfterReconnect.get(KEY_KEEPING_A_VALUE)).toContain(KEPT);
  });

  test('the server drops the value again for a key that still holds another value', () => {
    expectIncrementalWalkReachedBothKeys();

    const values = valuesOn(serverAfterHeal, KEY_KEEPING_A_VALUE);
    expect(values).not.toContain(REMOVED);
    expect(values).toContain(KEPT);
  });

  test('the server drops the value again for a key the client holds no live record for', () => {
    expectIncrementalWalkReachedBothKeys();

    expect(valuesOn(serverAfterHeal, KEY_EMPTIED)).toEqual([]);
  });
});
