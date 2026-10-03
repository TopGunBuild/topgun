/**
 * Integration: what an OR-Map client pushes during a Merkle walk against the
 * real Rust server.
 *
 * The server keeps tombstones per key and unions whatever an ORMAP_PUSH_DIFF
 * entry carries into that key's set. A client that pushes tombstones belonging
 * to another key makes the server's per-key set depend on push history, so the
 * two sides can never agree on that key's leaf (TG-MRK-001). The scenario below
 * forces a real divergence on one key and inspects the frame the client sends
 * back for it.
 *
 * Assertions are made on the captured outbound frame, not on server state: the
 * push has no response, and the server handles frames of one connection
 * concurrently, so nothing can be ordered after the push without waiting. The
 * server unions the pushed tombstones verbatim, so the frame decides the outcome.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncEngine, SyncState } from '@topgunbuild/client';

import { MerkleSyncHandler } from '../../packages/client/src/sync/MerkleSyncHandler';
import { ORMapSyncHandler } from '../../packages/client/src/sync/ORMapSyncHandler';
import { WebSocketManager } from '../../packages/client/src/sync/WebSocketManager';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createORRecord,
  waitUntil,
  MemoryStorageAdapter,
  SpawnedServer,
  TestClient,
} from './helpers';

const MAP_NAME = 'merkle_comparability_or';
const KEY_A = 'key-a';
const KEY_B = 'key-b';

/** Upper bound on any single setup wait. It only ever turns a hang into a failure. */
const SIGNAL_TIMEOUT_MS = 20_000;

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

interface PushDiffEntry {
  key: string;
  records: unknown[];
  tombstones: string[];
}

/**
 * Records events as they happen and lets a caller wait for the first one that
 * matches. Waiting on the event itself (rather than sleeping and looking) is
 * what keeps the outcome independent of timing; the deadline only bounds a run
 * in which the event never comes, and that case is reported as `null`.
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

describe('Integration: OR-Map push scoping during a forced-divergence walk', () => {
  let server: SpawnedServer | null = null;
  let dataDir: string | null = null;
  let reconnected: TopGunClient | null = null;

  const roots = new Captured<RootPayload>();
  const leaves = new Captured<LeafInvocation>();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
  let outbound: jest.SpyInstance<boolean, [message: any, key?: string]>;
  let recordedOps: jest.SpyInstance;

  let tagRemovedFromA: string;
  let tagRemovedFromB: string;
  let reconnectRoot: RootPayload | null = null;
  let leafForB: LeafInvocation | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
  let sentOnReconnect: any[] = [];

  /** Waits until every write recorded so far has been acknowledged by the server. */
  async function expectAllWritesAcked(client: TopGunClient): Promise<void> {
    const opIds: string[] = await Promise.all(recordedOps.mock.results.map((r) => r.value));
    for (const opId of opIds) {
      expect(await engineOf(client).waitForOpSynced(opId, SIGNAL_TIMEOUT_MS)).toBe('synced');
    }
  }

  function makeClient(port: number, storage: MemoryStorageAdapter): TopGunClient {
    const token = createTestToken('merkle-comparability-user', ['ADMIN']);
    return new TopGunClient({
      serverUrl: `ws://localhost:${port}/ws`,
      storage,
      auth: { getToken: async () => token },
      backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
    });
  }

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-merkle-comparability-'));

    // The write-behind flush interval is set far above this test's timeout so that no staged write
    // is flushed while the test runs. The durability watermark therefore stays 0, tombstone protection
    // never activates, and the reconnect cannot be routed to a full-resync REPLACE, which would clear the
    // client's map-wide tombstone set and make the pushed tombstones depend on response ordering. The null
    // backend is not used because it reports every stamp as durable at once, which activates protection
    // at the first remove. The fullResync assertion below fails if protection ever engages here.
    server = await spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
        TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000',
        TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '600000',
      },
    });

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

    outbound = jest.spyOn(WebSocketManager.prototype, 'sendMessage');
    recordedOps = jest.spyOn(SyncEngine.prototype, 'recordOperation');

    // --- The client builds its state and has every write acknowledged. ---
    //
    // The order is part of the setup: the remove on key A is the first remove the
    // server sees, and key A is never written again. With no flush tick, a staged
    // write only resolves when a later write to the same key supersedes it, so
    // that first remove stays pending for the whole test and pins the durability
    // watermark at 0.
    const storage = new MemoryStorageAdapter();
    const first = makeClient(server.port, storage);
    await first.start();
    await waitUntil(() => first.getConnectionState() === SyncState.CONNECTED, SIGNAL_TIMEOUT_MS);

    const map = first.getORMap(MAP_NAME);

    map.add(KEY_A, 'a1');
    await expectAllWritesAcked(first);
    [tagRemovedFromA] = map.remove(KEY_A, 'a1');
    await expectAllWritesAcked(first);

    map.add(KEY_B, 'b1');
    map.add(KEY_B, 'b2');
    await expectAllWritesAcked(first);
    [tagRemovedFromB] = map.remove(KEY_B, 'b1');
    await expectAllWritesAcked(first);

    await first.close();

    // --- While the client is away, another writer changes key B on the server. ---
    // This is what makes the divergence real server state: the server's leaf for
    // key B now holds a tag the client has never seen, so the walk has to reach it.
    const writer = await createRustTestClient(server.port, {
      nodeId: 'divergence-writer',
      userId: 'divergence-writer',
      roles: ['ADMIN'],
    });
    try {
      await writer.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
      writer.messages.length = 0;
      writer.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'divergence-add',
          mapName: MAP_NAME,
          opType: 'OR_ADD',
          key: KEY_B,
          orRecord: createORRecord('b3', 'divergence-writer'),
        },
      });
      await writer.waitForMessage('OP_ACK', SIGNAL_TIMEOUT_MS);
    } finally {
      writer.close();
    }

    // --- The client comes back with its persisted state and syncs. ---
    // Only what happens on this connection is evidence for the assertions below.
    roots.reset();
    leaves.reset();
    outbound.mockClear();
    reconnected = makeClient(server.port, storage);
    await reconnected.start();

    reconnectRoot = await roots.first((root) => root.mapName === MAP_NAME, SIGNAL_TIMEOUT_MS);
    leafForB = await leaves.first(
      (leaf) =>
        leaf.payload.mapName === MAP_NAME && leaf.payload.entries.some((e) => e.key === KEY_B),
      SIGNAL_TIMEOUT_MS,
    );
    // The push for a leaf is sent inside the handler invocation that merged it,
    // so once that invocation settles the frame for key B has been captured.
    if (leafForB) await leafForB.done;

    sentOnReconnect = outbound.mock.calls.map((call) => call[0]);
  });

  afterAll(async () => {
    jest.restoreAllMocks();
    if (reconnected) await reconnected.close().catch(() => {});
    if (server) await server.cleanup().catch(() => {});
    // Removed only after the server has stopped: it holds the store and the WAL open.
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  function pushedEntriesForKeyB(): PushDiffEntry[] {
    return sentOnReconnect
      .filter((msg) => msg?.type === 'ORMAP_PUSH_DIFF' && msg.payload?.mapName === MAP_NAME)
      .flatMap((msg) => msg.payload.entries as PushDiffEntry[])
      .filter((entry) => entry.key === KEY_B);
  }

  /**
   * Everything the two tombstone assertions depend on. Each of them runs this
   * first, so neither can pass on a run that never produced the frame, or that
   * produced it through a full-resync REPLACE.
   */
  function expectIncrementalWalkReachedKeyB(): PushDiffEntry[] {
    expect(reconnectRoot).not.toBeNull();
    expect(
      reconnectRoot?.fullResync
        ? 'reconnect was routed to full resync — R4b precondition violated'
        : 'incremental sync',
    ).toBe('incremental sync');

    const bucketRequests = sentOnReconnect.filter(
      (msg) => msg?.type === 'ORMAP_MERKLE_REQ_BUCKET' && msg.payload?.mapName === MAP_NAME,
    );
    expect(bucketRequests.length).toBeGreaterThanOrEqual(1);

    expect(
      leafForB ? 'leaf response for key B' : 'no leaf response for key B — walk did not reach B',
    ).toBe('leaf response for key B');

    const entries = pushedEntriesForKeyB();
    expect(entries.length).toBeGreaterThanOrEqual(1);
    return entries;
  }

  test('the pushed entry for key B carries no tag that was removed from key A', () => {
    const entries = expectIncrementalWalkReachedKeyB();

    expect(tagRemovedFromA).toEqual(expect.any(String));
    for (const entry of entries) {
      expect(entry.tombstones).not.toContain(tagRemovedFromA);
    }
  });

  test('the pushed entry for key B carries the tag that was removed from key B', () => {
    const entries = expectIncrementalWalkReachedKeyB();

    expect(tagRemovedFromB).toEqual(expect.any(String));
    for (const entry of entries) {
      expect(entry.tombstones).toContain(tagRemovedFromB);
    }
  });
});

/**
 * A client that holds exactly what the server holds, and has nothing left to
 * send, should learn that from the root alone: one root per map, equal to its
 * own, and no bucket request or push after it.
 *
 * "Holds what the server holds" is a state, not "every write was acknowledged".
 * The server stores a client's OR add under a tag of its own and a client's LWW
 * put under a timestamp of its own, and the acknowledgement carries neither; the
 * writer learns them only from a sync walk. So the writing device is staged
 * through one reconciling reload before the reload that is asserted, and a
 * second device is staged by pulling everything through a walk.
 *
 * Every asserted reload is a new client over persisted storage, which claims no
 * confirmed epoch. The server answers such a device with a full resync once
 * tombstone protection is active, so the whole describe keeps protection dark
 * with a pin: the first remove the server sees is on a key of a map nothing
 * touches again. That holds because of three pieces of server code:
 *   - `WriteBehindDataStore::flushed_watermark` (write_behind.rs) returns the
 *     smallest still-pending write sequence, and with the flush interval set far
 *     above the test's duration a staged write only resolves when a later write
 *     to the same key supersedes it;
 *   - `FrontierState::compute_durable_epoch_watermark`
 *     (tombstone_frontier_impl.rs) walks the stamped epochs in ascending order
 *     and stops at the first whose recorded write sequence is above that value,
 *     so the pinned remove's epoch, the lowest one, keeps the result at 0;
 *   - `TombstoneFrontier::is_protection_active` is `refreshed_watermark() > 0`.
 *
 * The seed is written with the client clock held five seconds behind, so every
 * server timestamp compares greater than the client's own and the reconciling
 * walk adopts it by construction. That relies on nothing advancing the client
 * clock on the seeding connection. `SyncEngine` calls `hlc.update` in three
 * places: the `onTimestampUpdate` callbacks of the two sync handlers, reached
 * only from a root response, which answers a sync init, and no sync init is
 * sent on a connection that authenticated with no map open (`registerMap` sends
 * none either); and the tail of `handleServerMessage`, for a frame with a
 * top-level HLC `timestamp`, which none of the frames such a connection
 * receives carries (the wire structs keep it inside `payload`). A map merge
 * also advances the clock, and the seeding client has no subscription to
 * deliver one. The staging checks the outcome as well: every seeded stamp must
 * not exceed the lagged clock reading.
 */
describe('Integration: a converged client reconnects in sync', () => {
  // Underscores, not hyphens: the embedded store this describe runs on only
  // accepts map names made of letters, digits and underscores, and a write to
  // any other name fails there without an acknowledgement.
  const LWW_MAP = 'r3_lww';
  const OR_MAP = 'r3_or';
  const PIN_MAP = 'r3_pin';
  const LWW_KEYS = ['l1', 'l2', 'l3', 'l4', 'l5', 'l6', 'l7', 'l8'];
  /** Ends up with no live value: only its tombstones keep it in the tree. */
  const K1 = 'k1';
  /** Loses one of its two values. */
  const K2 = 'k2';
  const OTHER_OR_KEYS = ['k3', 'k4', 'k5', 'k6'];
  const OR_KEYS = [K1, K2, ...OTHER_OR_KEYS];
  const SEED_LAG_MS = 5_000;
  const STAGED = 'staged';
  const WALK_FRAME_TYPES = ['MERKLE_REQ_BUCKET', 'ORMAP_MERKLE_REQ_BUCKET', 'ORMAP_PUSH_DIFF'];

  /** Which client an observed call belongs to. */
  type Owner = 'first' | 'second' | 'third' | 'fresh' | 'fresh-reload' | 'unowned';

  interface HlcStamp {
    millis: number | bigint;
    counter: number | bigint;
    nodeId: string;
  }

  interface LocalMap {
    getMerkleTree(): { getRootHash(): number };
    getKeyTombstones?: (key: string) => Set<string>;
  }

  interface RootCapture {
    owner: Owner;
    mapName: string;
    rootHash: number;
    fullResync: boolean;
    /** The client's own root, read when the handler was entered. */
    localRoot: number | null;
    /** The tags attributed to K1, read at the same point (OR maps only). */
    k1Tombstones: string[] | null;
    /** Settles when the handler has made its decision and sent what follows from it. */
    done: Promise<void>;
  }

  interface LeafCapture {
    owner: Owner;
    seq: number;
    mapName: string;
    keys: string[];
    /** Served LWW timestamps by key; empty for an OR leaf. */
    served: Map<string, HlcStamp>;
    done: Promise<void>;
  }

  interface Sequenced {
    owner: Owner;
    seq: number;
  }

  interface SentFrame extends Sequenced {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
    msg: any;
  }

  interface CoverageCall extends Sequenced {
    mapName: string;
    epoch: number;
  }

  interface RestoreCapture extends Sequenced {
    mapName: string;
    done: Promise<void>;
  }

  interface ReloadEvidence {
    lwwRoot: RootCapture;
    orRoot: RootCapture;
    /** Types of the walk frames sent for the two maps up to the roots' decisions. */
    walkFrames: string[];
  }

  let server: SpawnedServer | null = null;
  let dataDir: string | null = null;
  let observer: TestClient | null = null;
  const opened: TopGunClient[] = [];

  // One counter for every observed call, so that calls seen by different spies
  // can be ordered against each other. Calls are attributed to a client by the
  // instance they ran on, not by when they ran: a handler of a client that was
  // already closed can still finish, and must not be counted for the next one.
  let seq = 0;
  const owners = new WeakMap<object, Owner>();
  const ownerOf = (instance: unknown): Owner =>
    (typeof instance === 'object' && instance !== null && owners.get(instance)) || 'unowned';

  const roots = new Captured<RootCapture>();
  const leaves = new Captured<LeafCapture>();
  const restores = new Captured<RestoreCapture>();
  const sent: SentFrame[] = [];
  const coverage: CoverageCall[] = [];
  let recordedOps: jest.SpyInstance;

  let writerStaging = 'writer staging did not run';
  let freshStaging = 'fresh-device staging did not run';
  /** The last sequence number that belongs to the reconciling reload. */
  let r1EndSeq = 0;
  let k1Removed: string[] = [];
  let r2: ReloadEvidence | null = null;
  let freshReload: ReloadEvidence | null = null;

  /** A staging step that did not reach its target state; the message names it. */
  function need(condition: unknown, message: string): asserts condition {
    if (!condition) throw new Error(message);
  }

  async function reach(condition: () => boolean, message: string): Promise<void> {
    try {
      await waitUntil(condition, SIGNAL_TIMEOUT_MS);
    } catch {
      throw new Error(message);
    }
  }

  const stampOf = (ts: HlcStamp | undefined): string =>
    ts ? `${Number(ts.millis)}:${Number(ts.counter)}:${ts.nodeId}` : 'none';

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
  const mapOfFrame = (msg: any): string | undefined => msg?.mapName ?? msg?.payload?.mapName;

  function localMapOf(handler: unknown, mapName: string): LocalMap | undefined {
    return (handler as { config: { getMap(name: string): LocalMap | undefined } }).config.getMap(
      mapName,
    );
  }

  function deferredToken(): { promise: Promise<string>; release: () => void } {
    const token = createTestToken('merkle-in-sync-user', ['ADMIN']);
    let release: () => void = () => {};
    const promise = new Promise<string>((resolve) => {
      release = () => resolve(token);
    });
    return { promise, release };
  }

  /**
   * The token is a promise the caller controls: a connection authenticates, and
   * so starts its sync round, only once the caller has released it.
   */
  function makeClient(
    owner: Owner,
    storage: MemoryStorageAdapter,
    token: Promise<string>,
  ): TopGunClient {
    if (!server) throw new Error('server is not running');
    const client = new TopGunClient({
      serverUrl: `ws://localhost:${server.port}/ws`,
      storage,
      auth: { getToken: () => token },
      backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
    });
    const engine = engineOf(client) as unknown as Record<string, object>;
    owners.set(engine, owner);
    for (const part of ['webSocketManager', 'merkleSyncHandler', 'orMapSyncHandler']) {
      need(engine[part], `client has no ${part} to observe`);
      owners.set(engine[part], owner);
    }
    opened.push(client);
    return client;
  }

  /** Waits until every write recorded so far has been acknowledged by the server. */
  async function expectAllWritesAcked(client: TopGunClient): Promise<void> {
    const opIds: string[] = await Promise.all(recordedOps.mock.results.map((r) => r.value));
    for (const opId of opIds) {
      const outcome = await engineOf(client).waitForOpSynced(opId, SIGNAL_TIMEOUT_MS);
      need(outcome === 'synced', `precondition: seeded write ${opId} ended as ${outcome}`);
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- inbound frames have no single static shape
  function observedEvents(): any[] {
    if (!observer) return [];
    return observer.messages
      .filter((m) => m.type === 'SERVER_EVENT' || m.type === 'SERVER_BATCH_EVENT')
      .flatMap((m) => (Array.isArray(m.payload?.events) ? m.payload.events : [m.payload]))
      .filter((event) => event?.mapName === OR_MAP);
  }

  const observerSawAdd = (tag: string): boolean =>
    observedEvents().some((e) => e.eventType === 'OR_ADD' && e.orRecord?.tag === tag);
  const observerSawRemove = (tag: string): boolean =>
    observedEvents().some((e) => e.eventType === 'OR_REMOVE' && e.orTag === tag);

  const leavesOf = (owner: Owner, mapName: string): LeafCapture[] =>
    leaves.items.filter((leaf) => leaf.owner === owner && leaf.mapName === mapName);

  /** Resolves once the leaves a client has handled for a map list every key, and have been applied. */
  async function leavesCovering(
    owner: Owner,
    mapName: string,
    keys: string[],
    message: string,
  ): Promise<LeafCapture[]> {
    const covered = (): boolean => {
      const seen = new Set(leavesOf(owner, mapName).flatMap((leaf) => leaf.keys));
      return keys.every((key) => seen.has(key));
    };
    need(await leaves.first(covered, SIGNAL_TIMEOUT_MS), message);
    const handled = leavesOf(owner, mapName);
    await Promise.all(handled.map((leaf) => leaf.done));
    return handled;
  }

  /** Every key's stored record carries the timestamp the server served for it. */
  async function expectServedLwwTimestamps(
    owner: Owner,
    storage: MemoryStorageAdapter,
    message: string,
  ): Promise<void> {
    const handled = await leavesCovering(
      owner,
      LWW_MAP,
      LWW_KEYS,
      `precondition: the walk did not deliver every ${LWW_MAP} key`,
    );
    for (const key of LWW_KEYS) {
      const served = handled.map((leaf) => leaf.served.get(key)).filter(Boolean);
      const stored = await storage.get(`${LWW_MAP}:${key}`);
      need(stampOf(stored?.timestamp) === stampOf(served[served.length - 1]), message);
    }
  }

  /**
   * A new client over existing storage. The LWW map is opened by the test and
   * has to be whole before the connection authenticates, because its sync init
   * is sent right after; the OR map is left unopened, so the client restores it
   * itself, items and attribution, before that map's init.
   */
  async function reload(owner: Owner, storage: MemoryStorageAdapter): Promise<TopGunClient> {
    const gate = deferredToken();
    const client = makeClient(owner, storage, gate.promise);
    await client.start();
    const lww = client.getMap<string, string>(LWW_MAP);
    await reach(
      () => LWW_KEYS.every((key) => lww.getRecord(key) !== undefined),
      `precondition: ${LWW_MAP} was not restored from storage`,
    );
    need(
      !sent.some((frame) => frame.owner === owner && frame.msg?.type === 'SYNC_INIT'),
      'precondition: the sync round started before the LWW map was restored',
    );
    gate.release();
    return client;
  }

  /** What a reload's roots said, and what the client sent for the two maps up to their decisions. */
  async function observeReload(owner: Owner): Promise<ReloadEvidence> {
    const lwwRoot = await roots.first(
      (root) => root.owner === owner && root.mapName === LWW_MAP,
      SIGNAL_TIMEOUT_MS,
    );
    need(lwwRoot, `precondition: no SYNC_RESP_ROOT for ${LWW_MAP} on the reload`);
    const orRoot = await roots.first(
      (root) => root.owner === owner && root.mapName === OR_MAP,
      SIGNAL_TIMEOUT_MS,
    );
    need(orRoot, `precondition: no ORMAP_SYNC_RESP_ROOT for ${OR_MAP} on the reload`);
    // A mismatch sends its first request inside the root handler's invocation,
    // so once both have settled an in-sync outcome cannot change any more.
    await lwwRoot.done;
    await orRoot.done;
    const walkFrames = sent
      .filter((frame) => frame.owner === owner)
      .filter((frame) => WALK_FRAME_TYPES.includes(frame.msg?.type))
      .filter((frame) => [LWW_MAP, OR_MAP].includes(mapOfFrame(frame.msg) ?? ''))
      .map((frame) => frame.msg.type as string);
    return { lwwRoot, orRoot, walkFrames };
  }

  async function pinProtectionDark(port: number): Promise<void> {
    const pin = await createRustTestClient(port, {
      nodeId: 'r3-pin-writer',
      userId: 'r3-pin-writer',
      roles: ['ADMIN'],
    });
    try {
      await pin.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
      const record = createORRecord('pinned', 'r3-pin-writer');
      pin.messages.length = 0;
      pin.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'r3-pin-add',
          mapName: PIN_MAP,
          opType: 'OR_ADD',
          key: 'pin',
          orRecord: record,
        },
      });
      await pin.waitForMessage('OP_ACK', SIGNAL_TIMEOUT_MS);
      pin.messages.length = 0;
      pin.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'r3-pin-remove',
          mapName: PIN_MAP,
          opType: 'OR_REMOVE',
          key: 'pin',
          orTag: record.tag,
        },
      });
      await pin.waitForMessage('OP_ACK', SIGNAL_TIMEOUT_MS);
    } finally {
      pin.close();
    }
  }

  /** The writing device: seed, reconcile, remove, then the reload that is asserted. */
  async function stageWriter(port: number): Promise<void> {
    // The pin comes before any first-party client connects: its remove has to
    // be the first one the server sees.
    await pinProtectionDark(port);

    // The observer is how the staging learns what the server has taken in: it
    // is sent an event for every tag a push introduces and for every remove.
    observer = await createRustTestClient(port, {
      nodeId: 'r3-observer',
      userId: 'r3-observer',
      roles: ['ADMIN'],
    });
    await observer.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
    observer.send({
      type: 'QUERY_SUB',
      payload: { queryId: 'r3-observer-or', mapName: OR_MAP, query: {} },
    });
    await observer.waitForMessage('QUERY_RESP', SIGNAL_TIMEOUT_MS);

    // --- Seed: online, with no sync round on the connection. ---
    const storage = new MemoryStorageAdapter();
    const first = makeClient('first', storage, deferredTokenReleased());
    await first.start();
    await reach(
      () => first.getConnectionState() === SyncState.CONNECTED,
      'precondition: the seeding client did not connect',
    );

    const realNow = Date.now.bind(Date);
    let laggedReading = 0;
    const clock = jest.spyOn(Date, 'now').mockImplementation(() => {
      laggedReading = realNow() - SEED_LAG_MS;
      return laggedReading;
    });
    const seededTags: string[] = [];
    // One write at a time, each acknowledged before the next. A write that is
    // still pending is sent again with the next one, and the server stores an
    // add it receives twice under two tags of its own, so a burst would leave a
    // value with a number of server tags that depends on when the
    // acknowledgements happened to arrive.
    const seed = async (write: () => { timestamp: HlcStamp; tag?: string }): Promise<void> => {
      const record = write();
      need(
        Number(record.timestamp.millis) <= laggedReading,
        'precondition: a seeded stamp is ahead of the lagged clock, so the server stamp need not win',
      );
      if (record.tag) seededTags.push(record.tag);
      await expectAllWritesAcked(first);
    };
    try {
      const lww = first.getMap<string, string>(LWW_MAP);
      for (const key of LWW_KEYS) {
        await seed(() => lww.set(key, `value-${key}`));
      }
      const or = first.getORMap<string, string>(OR_MAP);
      await seed(() => or.add(K1, 'v'));
      await seed(() => or.add(K2, 'v1'));
      await seed(() => or.add(K2, 'v2'));
      for (const key of OTHER_OR_KEYS) {
        await seed(() => or.add(key, 'v'));
      }
    } finally {
      clock.mockRestore();
    }
    need(
      !sent.some(
        (frame) =>
          frame.owner === 'first' && ['SYNC_INIT', 'ORMAP_SYNC_INIT'].includes(frame.msg?.type),
      ),
      'precondition: the seeding connection ran a sync round',
    );
    await first.close();

    // --- The reconciling reload: one walk per map, and the push after the OR one. ---
    const second = await reload('second', storage);
    await expectServedLwwTimestamps(
      'second',
      storage,
      'precondition: client kept its own LWW timestamp (TODO-736)',
    );
    // The server now holds the client's tag next to its own for every value,
    // and the client merged the server's tag before it pushed.
    await reach(
      () => seededTags.every(observerSawAdd),
      'precondition: the server did not take in every client tag from the reconciling push',
    );
    await Promise.all(leavesOf('second', OR_MAP).map((leaf) => leaf.done));
    r1EndSeq = seq;

    // --- Removes, online. ---
    const or = second.getORMap<string, string>(OR_MAP);
    const restored = await restores.first(
      (restore) =>
        restore.owner === 'second' && restore.mapName === OR_MAP && restore.seq > r1EndSeq,
      SIGNAL_TIMEOUT_MS,
    );
    need(restored, `precondition: ${OR_MAP} was not restored before the removes`);
    await restored.done;

    const removesStartSeq = seq;
    k1Removed = or.remove(K1, 'v');
    need(
      k1Removed.length === 2,
      `precondition: remove on ${K1} returned ${k1Removed.length} tags, not the client's and the server's`,
    );
    const k2Removed = or.remove(K2, 'v1');
    need(
      k2Removed.length === 2,
      `precondition: remove on ${K2} returned ${k2Removed.length} tags, not the client's and the server's`,
    );
    // The removes and their attribution are in storage once their ops are recorded.
    await Promise.all(recordedOps.mock.results.map((r) => r.value));
    // Closed on what the server broadcast, not on the ops reading as synced:
    // only the former shows the server applied each remove.
    await reach(
      () => [...k1Removed, ...k2Removed].every(observerSawRemove),
      'precondition: the server did not apply every remove',
    );
    need(
      !sent.some(
        (frame) =>
          frame.owner === 'second' &&
          frame.seq > removesStartSeq &&
          ['ORMAP_SYNC_INIT', 'CLIENT_APPLY_ACK'].includes(frame.msg?.type),
      ),
      'precondition: a sync round or an epoch confirmation ran between the removes and the close',
    );
    await second.close();

    // --- The asserted reload: nothing changed locally since the removes. ---
    const third = await reload('third', storage);
    r2 = await observeReload('third');
    await third.close();
  }

  /** A device that starts empty, pulls both maps through a walk, and reloads. */
  async function stageFreshDevice(): Promise<void> {
    const storage = new MemoryStorageAdapter();
    const gate = deferredToken();
    const fresh = makeClient('fresh', storage, gate.promise);
    await fresh.start();
    fresh.getMap<string, string>(LWW_MAP);
    const or = fresh.getORMap<string, string>(OR_MAP);
    const restored = await restores.first(
      (restore) => restore.owner === 'fresh' && restore.mapName === OR_MAP,
      SIGNAL_TIMEOUT_MS,
    );
    need(restored, `precondition: the fresh device did not finish opening ${OR_MAP}`);
    await restored.done;
    gate.release();

    await expectServedLwwTimestamps(
      'fresh',
      storage,
      'precondition: the fresh device does not hold the served LWW timestamps',
    );
    await leavesCovering(
      'fresh',
      OR_MAP,
      OR_KEYS,
      'precondition: tombstone-only key not delivered',
    );
    need(
      [...or.getKeyTombstones(K1)].sort().join() === [...k1Removed].sort().join(),
      'precondition: tombstone-only key not delivered',
    );
    await fresh.close();

    const reloaded = await reload('fresh-reload', storage);
    freshReload = await observeReload('fresh-reload');
    await reloaded.close();
  }

  function deferredTokenReleased(): Promise<string> {
    const gate = deferredToken();
    gate.release();
    return gate.promise;
  }

  beforeAll(async () => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-merkle-in-sync-'));

    // No staged write is flushed while the describe runs, which is what lets one
    // pinned remove keep tombstone protection dark throughout (see the comment
    // above the describe).
    server = await spawnRustServer({
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
        TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000',
        TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '600000',
      },
    });

    const originalOrRoot = ORMapSyncHandler.prototype.handleORMapSyncRespRoot;
    jest.spyOn(ORMapSyncHandler.prototype, 'handleORMapSyncRespRoot').mockImplementation(function (
      this: ORMapSyncHandler,
      payload,
    ) {
      const local = localMapOf(this, payload.mapName);
      const capture = {
        owner: ownerOf(this),
        mapName: payload.mapName,
        rootHash: Number(payload.rootHash),
        fullResync: Boolean((payload as RootPayload).fullResync),
        localRoot: local ? local.getMerkleTree().getRootHash() : null,
        k1Tombstones: local?.getKeyTombstones ? [...local.getKeyTombstones(K1)].sort() : null,
      };
      const done = originalOrRoot.call(this, payload);
      roots.record({ ...capture, done });
      return done;
    });

    const originalLwwRoot = MerkleSyncHandler.prototype.handleSyncRespRoot;
    jest.spyOn(MerkleSyncHandler.prototype, 'handleSyncRespRoot').mockImplementation(function (
      this: MerkleSyncHandler,
      payload,
    ) {
      const local = localMapOf(this, payload.mapName);
      const capture = {
        owner: ownerOf(this),
        mapName: payload.mapName,
        rootHash: Number(payload.rootHash),
        fullResync: Boolean((payload as RootPayload).fullResync),
        localRoot: local ? local.getMerkleTree().getRootHash() : null,
        k1Tombstones: null,
      };
      const done = originalLwwRoot.call(this, payload);
      roots.record({ ...capture, done });
      return done;
    });

    const originalOrLeaf = ORMapSyncHandler.prototype.handleORMapSyncRespLeaf;
    jest.spyOn(ORMapSyncHandler.prototype, 'handleORMapSyncRespLeaf').mockImplementation(function (
      this: ORMapSyncHandler,
      payload,
    ) {
      const capture = {
        owner: ownerOf(this),
        seq: ++seq,
        mapName: payload.mapName,
        keys: payload.entries.map((entry) => entry.key),
        served: new Map<string, HlcStamp>(),
      };
      const done = originalOrLeaf.call(this, payload);
      leaves.record({ ...capture, done });
      return done;
    });

    const originalLwwLeaf = MerkleSyncHandler.prototype.handleSyncRespLeaf;
    jest.spyOn(MerkleSyncHandler.prototype, 'handleSyncRespLeaf').mockImplementation(function (
      this: MerkleSyncHandler,
      payload,
    ) {
      const capture = {
        owner: ownerOf(this),
        seq: ++seq,
        mapName: payload.mapName,
        keys: payload.records.map((entry) => entry.key),
        served: new Map<string, HlcStamp>(
          payload.records.map((entry) => [entry.key, entry.record.timestamp]),
        ),
      };
      const done = originalLwwLeaf.call(this, payload);
      leaves.record({ ...capture, done });
      return done;
    });

    const originalSend = WebSocketManager.prototype.sendMessage;
    jest.spyOn(WebSocketManager.prototype, 'sendMessage').mockImplementation(function (
      this: WebSocketManager,
      message,
      key,
    ) {
      sent.push({ owner: ownerOf(this), seq: ++seq, msg: message });
      return originalSend.call(this, message, key);
    });

    // Private on the engine; reached through the prototype because it is the
    // one place every per-map epoch confirmation passes through.
    const enginePrototype = SyncEngine.prototype as unknown as {
      applyMapCoverage(mapName: string, epoch: number): void;
    };
    const originalCoverage = enginePrototype.applyMapCoverage;
    jest.spyOn(enginePrototype, 'applyMapCoverage').mockImplementation(function (
      this: unknown,
      mapName,
      epoch,
    ) {
      coverage.push({ owner: ownerOf(this), seq: ++seq, mapName, epoch });
      return originalCoverage.call(this, mapName, epoch);
    });

    const originalEndRestore = SyncEngine.prototype.endOrMapRestore;
    jest.spyOn(SyncEngine.prototype, 'endOrMapRestore').mockImplementation(function (
      this: SyncEngine,
      mapName,
      attributionRestored,
    ) {
      const capture = { owner: ownerOf(this), seq: ++seq, mapName };
      const done = originalEndRestore.call(this, mapName, attributionRestored);
      restores.record({ ...capture, done });
      return done;
    });

    recordedOps = jest.spyOn(SyncEngine.prototype, 'recordOperation');

    // A staging step that misses its target is kept as a named outcome for the
    // tests to report, instead of failing the hook with a bare error.
    try {
      await stageWriter(server.port);
      writerStaging = STAGED;
    } catch (err) {
      writerStaging = err instanceof Error ? err.message : String(err);
    }
    if (writerStaging === STAGED) {
      try {
        await stageFreshDevice();
        freshStaging = STAGED;
      } catch (err) {
        freshStaging = err instanceof Error ? err.message : String(err);
      }
    } else {
      freshStaging = `the writing device was not staged: ${writerStaging}`;
    }
  }, 180_000);

  afterAll(async () => {
    jest.restoreAllMocks();
    for (const client of opened) await client.close().catch(() => {});
    if (observer) observer.close();
    if (server) await server.cleanup().catch(() => {});
    // Removed only after the server has stopped: it holds the store and the WAL open.
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
  });

  /**
   * A reload answered with a full resync would replace the client's state and
   * say nothing about the in-sync branch, so every test rules it out first.
   */
  function expectNoFullResync(): void {
    expect(
      roots.items.some((root) => root.fullResync)
        ? 'R3 precondition violated: reconnect was routed to full resync'
        : 'incremental sync',
    ).toBe('incremental sync');
  }

  function expectNonZeroRoots(reloaded: ReloadEvidence): void {
    for (const root of [reloaded.lwwRoot, reloaded.orRoot]) {
      expect(root.rootHash).not.toBe(0);
      expect(root.localRoot).not.toBe(0);
    }
  }

  function expectInSync(reloaded: ReloadEvidence): void {
    expect({ lww: reloaded.lwwRoot.rootHash, or: reloaded.orRoot.rootHash }).toEqual({
      lww: reloaded.lwwRoot.localRoot,
      or: reloaded.orRoot.localRoot,
    });
    expect(reloaded.walkFrames).toEqual([]);
  }

  test('R3a: converged client reconnect takes the in-sync branch for LWW and OR', () => {
    expect(writerStaging).toBe(STAGED);
    if (!r2) throw new Error('the asserted reload left no evidence');
    expectNoFullResync();
    expectNonZeroRoots(r2);

    expectInSync(r2);
  });

  test('R3b: fresh device after a full walk reconnects in sync', () => {
    expect(freshStaging).toBe(STAGED);
    if (!freshReload) throw new Error('the fresh device reload left no evidence');
    expectNoFullResync();
    expectNonZeroRoots(freshReload);

    expectInSync(freshReload);
  });

  test("R3c: attribution restored from its buckets yields the server's root", () => {
    expect(writerStaging).toBe(STAGED);
    if (!r2) throw new Error('the asserted reload left no evidence');
    expectNoFullResync();

    // The OR map was restored by the client itself, before its sync init: the
    // tombstone-only key already carries both removed tags when the root arrives.
    expect(k1Removed).toHaveLength(2);
    expect(r2.orRoot.k1Tombstones).toEqual([...k1Removed].sort());
    expect(r2.orRoot.rootHash).not.toBe(0);
    expect(r2.orRoot.localRoot).not.toBe(0);

    expect(r2.orRoot.rootHash).toBe(r2.orRoot.localRoot);
    expect(r2.walkFrames.filter((type) => type !== 'MERKLE_REQ_BUCKET')).toEqual([]);
  });

  test('R11: the covering epoch is acknowledged only after the walk has drained', () => {
    expect(writerStaging).toBe(STAGED);
    const duringR1 = <T extends Sequenced>(calls: T[]): T[] =>
      calls.filter((call) => call.owner === 'second' && call.seq <= r1EndSeq);

    const orLeaves = duringR1(leaves.items).filter((leaf) => leaf.mapName === OR_MAP);
    const acks = duringR1(sent).filter((frame) => frame.msg?.type === 'CLIENT_APPLY_ACK');
    expect(
      orLeaves.length >= 2
        ? 'walk with several leaves'
        : `precondition: the reconciling walk handled ${orLeaves.length} leaf responses for ${OR_MAP}, at least 2 are needed`,
    ).toBe('walk with several leaves');
    expect(
      acks.length >= 1
        ? 'epoch confirmed'
        : 'precondition: the reconciling reload sent no CLIENT_APPLY_ACK, so no epoch was conveyed or the barrier was closed',
    ).toBe('epoch confirmed');

    // One walk, one confirmation: the map's epoch is reported once, when the
    // last leaf has been applied, and not once per leaf. Only calls that carry
    // an epoch are confirmations: the events the server broadcasts for the
    // walk's own pushes pass through the same method with no epoch, and those
    // calls change nothing.
    const confirmations = duringR1(coverage).filter(
      (call) => call.mapName === OR_MAP && call.epoch > 0,
    );
    expect(confirmations).toHaveLength(1);

    const lastLeafCall = Math.max(...orLeaves.map((leaf) => leaf.seq));
    expect(acks[0].seq).toBeGreaterThan(lastLeafCall);
  });
});
