/**
 * Integration: a local write survives an operation acknowledgement that names
 * no batch the client sent, against the real Rust server.
 *
 * A "stray" acknowledgement is an OP_ACK sent in answer to a frame that is not
 * an operation batch (a search unsubscribe, an OR-Map push diff, an empty
 * batch). Its `lastId` is a server-wide call counter, unrelated to any client
 * op id. A client that reads it as "everything up to `lastId` is applied"
 * durably deletes pending writes the server never received.
 *
 * WHAT A GREEN RUN OF THIS FILE DOES AND DOES NOT PROVE. CI runs it in regime 0
 * only (`TOPGUN_TEST_EXPECT_STRAY_OP_ACK` unset): a server that sends no stray
 * frame, with the client of the same tree. In that regime L1 and L2 pass with
 * or without the client-side rule, because no stray frame exists to mishandle.
 * So a green regime-0 run is NOT proof of the client rule: the server half is
 * guarded by the Rust tests and the compile-time guard on the acknowledgement
 * variant, the client rule by the SyncEngine unit tests, and the two
 * cross-version legs (new client + old server, old client + new server) are
 * one-off runs recorded with the sha256 of the binary used, not CI jobs.
 *
 * Regime flag: `TOPGUN_TEST_EXPECT_STRAY_OP_ACK=1` states that the server under
 * test DOES send the stray frame. Every scenario then waits for the frame as an
 * event and fails its precondition if none arrives. Unset or `0` states that it
 * does not; the absence checks are then bounded negatives, supportive only. A
 * flag that contradicts the binary fails loudly either way.
 *
 * Ordering is by event throughout. The server handles every inbound frame in a
 * task of its own, so answers to two frames of one connection are not ordered
 * and a later round trip proves nothing about an earlier frame. Timeouts here
 * only turn a hang into a failure.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncEngine, SyncState } from '@topgunbuild/client';
import { deserialize, serialize } from '@topgunbuild/core';

import { ORMapSyncHandler } from '../../packages/client/src/sync/ORMapSyncHandler';
import { WebSocketManager } from '../../packages/client/src/sync/WebSocketManager';
import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createLWWRecord,
  createORRecord,
  completeMerkleSync,
  waitUntil,
  MemoryStorageAdapter,
  SpawnedServer,
  TestClient,
} from './helpers';

const REGIME_FLAG = 'TOPGUN_TEST_EXPECT_STRAY_OP_ACK';
const regimeValue = process.env[REGIME_FLAG] ?? '';
if (!['', '0', '1'].includes(regimeValue)) {
  // A typo must not silently select the regime in which nothing is awaited.
  throw new Error(`${REGIME_FLAG} must be "1", "0" or unset, got "${regimeValue}"`);
}
/** Regime 1: the server under test is expected to send the stray frame. */
const EXPECT_STRAY = regimeValue === '1';
const inStrayRegime = EXPECT_STRAY ? test : test.skip;
const inNoStrayRegime = EXPECT_STRAY ? test.skip : test;

/** Upper bound on any single setup wait. It only ever turns a hang into a failure. */
const SIGNAL_TIMEOUT_MS = 20_000;
/** How long a scenario waits for a stray frame before reporting that none came. */
const STRAY_TIMEOUT_MS = 10_000;
/**
 * Operations a raw connection sends before the SDK client connects. Each one
 * advances the server-wide call counter, which is what a stray frame carries as
 * its `lastId`. The SDK client's op ids stay below 10, so a stray frame then
 * always covers the pending op. Without this the counter could be smaller than
 * the pending id and a client that mishandles the frame would lose nothing.
 */
const WARM_UP_OPS = 64;

const WARM_UP_MAP = 'stray_op_ack_warm_up';

interface AckPayload {
  lastId: string;
  achievedLevel?: string | null;
  results?: unknown[] | null;
}

interface OutboundFrame {
  type?: string;
  payload?: {
    mapName?: string;
    ops?: Array<{ id?: string | number }>;
  };
}

interface SentFrame {
  frame: OutboundFrame;
  /** True when the frame was recorded and then withheld from the socket. */
  dropped: boolean;
}

interface InboundFrame {
  type?: string;
  payload?: AckPayload;
}

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

/** What one SDK scenario observed. The tests only read it. */
interface ScenarioObservations {
  /** Op id of the write whose batch was withheld from the socket. */
  lostOpId: string | null;
  /** A withheld OP_BATCH carried the op on the connection the trigger was sent on. */
  droppedBatchCarriedLostOp: boolean;
  /** The frame the server answers with a stray acknowledgement left the client. */
  triggerLeft: boolean;
  /** Regime 1 only: the stray acknowledgement the scenario waited for. */
  stray: AckPayload | null;
  /** Taken after the trigger was sent and, in regime 1, after its answer was handled. */
  pendingCount: number;
  pendingIdsInStorage: string[];
  markOpsSyncedArgs: number[];
  /** Outcome of the write made after the frame loss ended. */
  nextWrite: string | null;
  /** Last op id of every OP_BATCH the client recorded as sent, withheld ones included. */
  batchLastIds: string[];
  /** Every acknowledgement without an achieved level seen by the end of the scenario. */
  acksWithoutLevel: AckPayload[];
  /** The server's records for the map, read over a connection of its own. */
  serverValues: Map<string, unknown>;
}

function emptyObservations(): ScenarioObservations {
  return {
    lostOpId: null,
    droppedBatchCarriedLostOp: false,
    triggerLeft: false,
    stray: null,
    pendingCount: -1,
    pendingIdsInStorage: [],
    markOpsSyncedArgs: [],
    nextWrite: null,
    batchLastIds: [],
    acksWithoutLevel: [],
    serverValues: new Map(),
  };
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

const isAbsent = (value: unknown): boolean => value === undefined || value === null;

/** The stray shape: an acknowledgement that names neither a level nor per-op results. */
const isStray = (ack: AckPayload): boolean => isAbsent(ack.achievedLevel) && isAbsent(ack.results);

const hasNoLevel = (ack: AckPayload): boolean => isAbsent(ack.achievedLevel);

const isNumeric = (id: string): boolean => /^\d+$/.test(id);

/**
 * Lets everything already queued on the promise job queue run.
 *
 * The client queues the durable op-log delete on a promise chain when it
 * handles an acknowledgement. The in-memory storage adapter does no I/O, so the
 * chain needs no further event to finish: it has run by the next turn of the
 * event loop. This is not a wait for the server.
 */
function drainPromiseJobs(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve));
}

/** A raw protocol connection whose inbound frames can be awaited one by one. */
interface RawConnection {
  client: TestClient;
  frames: Captured<InboundFrame>;
}

async function connectRaw(port: number, name: string): Promise<RawConnection> {
  const client = await createRustTestClient(port, {
    nodeId: name,
    userId: name,
    roles: ['ADMIN'],
  });
  // Attached in the same turn the socket opened in, so no frame is missed.
  const frames = new Captured<InboundFrame>();
  client.ws.on('message', (data: ArrayBuffer | Buffer) => {
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : data;
    frames.record(deserialize(bytes as Uint8Array) as InboundFrame);
  });
  await client.waitForMessage('AUTH_ACK', SIGNAL_TIMEOUT_MS);
  return { client, frames };
}

const isAckFor =
  (opId: string) =>
  (frame: InboundFrame): boolean =>
    frame.type === 'OP_ACK' && frame.payload?.lastId === opId;

/**
 * Advances the server-wide call counter past any op id the SDK client will use.
 *
 * Every one of the acknowledgements is awaited, not only the last: the server
 * handles the frames concurrently, so the last one being answered does not show
 * that the others have been counted yet.
 */
async function warmUpCallCounter(port: number): Promise<void> {
  const raw = await connectRaw(port, 'stray-op-ack-warm-up');
  try {
    const opIds = Array.from({ length: WARM_UP_OPS }, (_, i) => `warm-up-${i + 1}`);
    for (const id of opIds) {
      raw.client.send({
        type: 'CLIENT_OP',
        payload: {
          id,
          mapName: WARM_UP_MAP,
          opType: 'PUT',
          key: id,
          record: createLWWRecord({ warmUp: true }, 'stray-op-ack-warm-up'),
        },
      });
    }
    for (const id of opIds) {
      const ack = await raw.frames.first(isAckFor(id), SIGNAL_TIMEOUT_MS);
      if (!ack) throw new Error(`warm-up write ${id} was not acknowledged`);
    }
  } finally {
    raw.client.close();
  }
}

/** Reads what the server holds for an LWW map, over a fresh connection's Merkle walk. */
async function readServerMap(port: number, mapName: string): Promise<Map<string, unknown>> {
  const raw = await connectRaw(port, `stray-op-ack-reader-${mapName}`);
  try {
    const records = await completeMerkleSync(raw.client, mapName, SIGNAL_TIMEOUT_MS);
    return new Map(Array.from(records, ([key, record]) => [key, record.value]));
  } finally {
    raw.client.close();
  }
}

function makeClient(port: number, storage: MemoryStorageAdapter): TopGunClient {
  const token = createTestToken('stray-op-ack-user', ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage,
    auth: { getToken: async () => token },
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

async function connected(client: TopGunClient): Promise<void> {
  await client.start();
  await waitUntil(() => client.getConnectionState() === SyncState.CONNECTED, SIGNAL_TIMEOUT_MS);
}

function engineOf(client: TopGunClient): SyncEngine {
  return (client as unknown as { syncEngine: SyncEngine }).syncEngine;
}

/** 4-byte big-endian length prefix, as the server's batch unpacking expects. */
function lenPrefix(n: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32BE(n, 0);
  return b;
}

/** Wraps inner-message bytes as a single-entry batch `data` blob. */
function packBatchData(inner: Uint8Array): Buffer {
  return Buffer.concat([lenPrefix(inner.length), Buffer.from(inner)]);
}

/**
 * Hand-encodes the envelope `fixmap{ type:"BATCH", count:1, data:<bin> }`.
 *
 * The client `serialize()` walks a binary `data` field as if its bytes were
 * object keys, so the envelope is encoded directly to keep `data` a MsgPack
 * `bin` blob, which is what the server's batch body expects.
 */
function encodeBatchFrame(data: Uint8Array): Uint8Array {
  const head = Buffer.concat([
    Buffer.from([0x83]), // fixmap, 3 entries
    Buffer.from([0xa4]),
    Buffer.from('type'),
    Buffer.from([0xa5]),
    Buffer.from('BATCH'),
    Buffer.from([0xa5]),
    Buffer.from('count'),
    Buffer.from([0x01]),
    Buffer.from([0xa4]),
    Buffer.from('data'),
    Buffer.from([0xc6]), // bin32 marker; 4-byte big-endian length follows
  ]);
  return new Uint8Array(Buffer.concat([head, lenPrefix(data.length), Buffer.from(data)]));
}

describe('Integration: an acknowledgement that names no batch the client sent', () => {
  const sent = new Captured<SentFrame>();
  const acks = new Captured<AckPayload>();
  const roots = new Captured<RootPayload>();
  const leaves = new Captured<LeafInvocation>();
  let recordedOps: jest.SpyInstance;

  /**
   * While true, an OP_BATCH is recorded and reported as sent without reaching
   * the socket. That is a frame lost in transit: the client believes it sent
   * the batch and the server never receives it.
   */
  let holdOpBatches = false;

  const carriesOp =
    (opId: string | null, dropped: boolean) =>
    (item: SentFrame): boolean =>
      item.dropped === dropped &&
      item.frame.type === 'OP_BATCH' &&
      (item.frame.payload?.ops ?? []).some((op) => String(op.id) === opId);

  const lastIdsOfRecordedBatches = (): string[] =>
    sent.items
      .filter((item) => item.frame.type === 'OP_BATCH')
      .map((item) => item.frame.payload?.ops ?? [])
      .filter((ops) => ops.length > 0)
      .map((ops) => String(ops[ops.length - 1].id));

  /** Waits until every write recorded so far has been acknowledged by the server. */
  async function expectAllWritesAcked(client: TopGunClient): Promise<void> {
    const opIds: string[] = await Promise.all(recordedOps.mock.results.map((r) => r.value));
    for (const opId of opIds) {
      expect(await engineOf(client).waitForOpSynced(opId, SIGNAL_TIMEOUT_MS)).toBe('synced');
    }
  }

  /**
   * The part every SDK scenario shares once its trigger frame has left: wait
   * for the stray answer where one is expected, record what the client then
   * holds, end the frame loss, make one more write and read the server.
   */
  async function observeAfterTrigger(
    observed: ScenarioObservations,
    client: TopGunClient,
    storage: MemoryStorageAdapter,
    markOpsSynced: jest.SpyInstance,
    port: number,
    mapName: string,
    nextKey: string,
    nextValue: string,
  ): Promise<void> {
    if (EXPECT_STRAY) {
      observed.stray = await acks.first(isStray, STRAY_TIMEOUT_MS);
    }
    await drainPromiseJobs();

    observed.pendingCount = client.getPendingOpsCount();
    observed.pendingIdsInStorage = (await storage.getPendingOps()).map((op) => String(op.id));
    observed.markOpsSyncedArgs = markOpsSynced.mock.calls.map((call) => Number(call[0]));

    holdOpBatches = false;
    client.getMap<string, string>(mapName).set(nextKey, nextValue);
    observed.nextWrite = await client.confirmWrite(mapName, nextKey, SIGNAL_TIMEOUT_MS);

    observed.serverValues = await readServerMap(port, mapName);
    observed.batchLastIds = lastIdsOfRecordedBatches();
    observed.acksWithoutLevel = acks.items.filter(hasNoLevel);
  }

  /** What the stray frame must look like for the scenario to have tested anything. */
  function expectStrayCoversThePendingOp(observed: ScenarioObservations): void {
    expect(
      observed.stray
        ? 'stray acknowledgement observed'
        : `no acknowledgement without a level arrived — ${REGIME_FLAG}=1 contradicts this server`,
    ).toBe('stray acknowledgement observed');
    const lastId = observed.stray?.lastId ?? '';
    expect(isNumeric(lastId)).toBe(true);
    expect(Number(lastId)).toBeGreaterThan(Number(observed.lostOpId));
    expect(observed.batchLastIds).not.toContain(lastId);
  }

  beforeAll(() => {
    const originalSend = WebSocketManager.prototype.sendMessage;
    jest.spyOn(WebSocketManager.prototype, 'sendMessage').mockImplementation(function (
      this: WebSocketManager,
      message,
      key,
    ) {
      const frame = message as OutboundFrame;
      const dropped = holdOpBatches && frame?.type === 'OP_BATCH';
      sent.record({ frame, dropped });
      if (dropped) return true;
      return originalSend.call(this, message, key);
    });

    // The handler is private; the engine reaches it through `this`, so a
    // prototype spy sees every acknowledgement the engine handles. The payload
    // is recorded AFTER the real handler returns, so a waiter resumes with the
    // handler's effects already in place.
    const enginePrototype = SyncEngine.prototype as unknown as {
      handleOpAck(message: { payload: AckPayload }): void;
    };
    const originalAck = enginePrototype.handleOpAck;
    jest.spyOn(enginePrototype, 'handleOpAck').mockImplementation(function (
      this: unknown,
      message,
    ) {
      try {
        originalAck.call(this, message);
      } finally {
        acks.record({ ...message.payload });
      }
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

    recordedOps = jest.spyOn(SyncEngine.prototype, 'recordOperation');
  });

  afterAll(() => {
    jest.restoreAllMocks();
  });

  // ------------------------------------------------------------------------
  // L1: the trigger is a search unsubscribe, sent on every handle dispose.
  // ------------------------------------------------------------------------
  describe('L1: a search handle is disposed while a write is pending', () => {
    const MAP = 'stray_op_ack_l1';
    const K_SANITY = 'key-sanity';
    const K_LOST = 'key-lost';
    const K_NEXT = 'key-next';
    const V_LOST = 'value-of-the-lost-write';
    const V_NEXT = 'value-of-the-next-write';

    let server: SpawnedServer | null = null;
    let client: TopGunClient | null = null;
    const observed = emptyObservations();
    let sanityWrite: string | null = null;

    beforeAll(async () => {
      server = await spawnRustServer();
      await warmUpCallCounter(server.port);

      const storage = new MemoryStorageAdapter();
      const markOpsSynced = jest.spyOn(storage, 'markOpsSynced');
      client = makeClient(server.port, storage);
      await connected(client);

      // --- 1. A write goes through end to end, so the connection is known good. ---
      const map = client.getMap<string, string>(MAP);
      map.set(K_SANITY, 'sanity');
      sanityWrite = await client.confirmWrite(MAP, K_SANITY, SIGNAL_TIMEOUT_MS);

      // --- 2. A live search subscription exists. ---
      const handle = client.searchSubscribe(MAP, 'sanity');

      // --- 3. A write whose batch is lost in transit: pending, and unapplied. ---
      await drainPromiseJobs();
      markOpsSynced.mockClear();
      recordedOps.mockClear();
      sent.reset();
      acks.reset();
      holdOpBatches = true;
      map.set(K_LOST, V_LOST);
      observed.lostOpId = await recordedOps.mock.results[0].value;
      observed.droppedBatchCarriedLostOp = sent.items.some(carriesOp(observed.lostOpId, true));

      // --- 4. The handle is disposed; its unsubscribe is what the server answers. ---
      handle.dispose();
      observed.triggerLeft = sent.items.some(
        (item) => !item.dropped && item.frame.type === 'SEARCH_UNSUB',
      );

      // --- 5-8. ---
      await observeAfterTrigger(
        observed,
        client,
        storage,
        markOpsSynced,
        server.port,
        MAP,
        K_NEXT,
        V_NEXT,
      );
    }, 180_000);

    afterAll(async () => {
      holdOpBatches = false;
      if (client) await client.close().catch(() => {});
      if (server) await server.cleanup().catch(() => {});
    });

    /** A pass of the decisive test means nothing unless the scenario reached its state. */
    function expectScenarioReachedItsState(): void {
      expect(sanityWrite).toBe('synced');
      expect(observed.lostOpId).toEqual(expect.any(String));
      expect(
        observed.droppedBatchCarriedLostOp
          ? 'the batch carrying the write was withheld'
          : 'no withheld OP_BATCH carried the write',
      ).toBe('the batch carrying the write was withheld');
      expect(
        observed.triggerLeft ? 'SEARCH_UNSUB left the client' : 'SEARCH_UNSUB was not sent',
      ).toBe('SEARCH_UNSUB left the client');
      expect(observed.nextWrite).toBe('synced');
      expect(observed.serverValues.get(K_SANITY)).toBe('sanity');
      expect(observed.serverValues.get(K_NEXT)).toBe(V_NEXT);
    }

    test('the server holds the write whose batch was lost in transit', () => {
      expect(observed.serverValues.get(K_LOST)).toBe(V_LOST);
      expectScenarioReachedItsState();
    });

    test('the scenario left the write pending and unapplied when the unsubscribe was sent', () => {
      expectScenarioReachedItsState();
    });

    inStrayRegime('the unsubscribe was answered with a stray acknowledgement', () => {
      expectStrayCoversThePendingOp(observed);
    });

    test('the write is still pending in memory after the unsubscribe', () => {
      expect(observed.pendingCount).toBe(1);
    });

    test('the write is still pending in storage and nothing was durably marked synced', () => {
      expect(observed.pendingIdsInStorage).toContain(observed.lostOpId);
      expect(observed.markOpsSyncedArgs).toEqual([]);
    });

    // A bounded negative: a stray frame slower than the whole scenario would be
    // missed. Supportive only; the decisive negatives are the server's own tests.
    inNoStrayRegime('no acknowledgement without a level reached the client', () => {
      expect(observed.acksWithoutLevel).toEqual([]);
    });
  });

  // ------------------------------------------------------------------------
  // L2: the trigger is the OR-Map push a reconnecting client's Merkle walk sends.
  // ------------------------------------------------------------------------
  describe('L2: a reconnect pushes an OR-Map diff while a write is pending', () => {
    const MAP = 'stray_op_ack_l2';
    const OR_MAP = 'stray_op_ack_l2_or';
    const KEY_B = 'key-b';
    const K_LOST = 'key-lost';
    const K_NEXT = 'key-next';
    const V_LOST = 'value-of-the-lost-write';
    const V_NEXT = 'value-of-the-next-write';

    let server: SpawnedServer | null = null;
    let dataDir: string | null = null;
    let first: TopGunClient | null = null;
    let second: TopGunClient | null = null;
    const observed = emptyObservations();
    let droppedBeforeReconnect = false;
    let reconnectRoot: RootPayload | null = null;
    let walkReachedKeyB = false;

    beforeAll(async () => {
      dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-stray-op-ack-'));

      // The write-behind flush interval is set far above the test's duration so
      // that no staged write is flushed while it runs. The durability watermark
      // then stays 0, tombstone protection never activates, and the reconnect is
      // an incremental walk rather than a full-resync REPLACE, which would not
      // push the leaf back. The null backend reports every stamp as durable at
      // once, so it is not used. The fullResync assertion below fails if the
      // reconnect is ever routed to a full resync.
      server = await spawnRustServer({
        env: {
          STORAGE_BACKEND: 'redb',
          TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
          TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
          TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000',
          TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '600000',
        },
      });
      await warmUpCallCounter(server.port);

      const storage = new MemoryStorageAdapter();
      const markOpsSynced = jest.spyOn(storage, 'markOpsSynced');

      // --- Session 1: OR adds, each acknowledged; then a write whose batch is lost. ---
      recordedOps.mockClear();
      first = makeClient(server.port, storage);
      await connected(first);
      const orMap = first.getORMap<string, string>(OR_MAP);
      // One add at a time: an add still waiting for its acknowledgement is sent
      // again with the next one, which would only add duplicate records.
      orMap.add(KEY_B, 'b1');
      await expectAllWritesAcked(first);
      orMap.add(KEY_B, 'b2');
      await expectAllWritesAcked(first);

      recordedOps.mockClear();
      sent.reset();
      holdOpBatches = true;
      first.getMap<string, string>(MAP).set(K_LOST, V_LOST);
      observed.lostOpId = await recordedOps.mock.results[0].value;
      droppedBeforeReconnect = sent.items.some(carriesOp(observed.lostOpId, true));
      await first.close();
      first = null;

      // --- While the client is away, another writer changes key B on the server. ---
      // The server's leaf for key B then holds a tag the client has never seen,
      // so the client's next walk has to reach that leaf and push its own side.
      const writer = await connectRaw(server.port, 'stray-op-ack-divergence-writer');
      try {
        writer.client.send({
          type: 'CLIENT_OP',
          payload: {
            id: 'divergence-add',
            mapName: OR_MAP,
            opType: 'OR_ADD',
            key: KEY_B,
            orRecord: createORRecord('b3', 'stray-op-ack-divergence-writer'),
          },
        });
        const ack = await writer.frames.first(isAckFor('divergence-add'), SIGNAL_TIMEOUT_MS);
        if (!ack) throw new Error('the divergence write was not acknowledged');
      } finally {
        writer.client.close();
      }

      // --- Session 2: same storage, frame loss still on. ---
      // The flush the client makes on authentication is withheld too, so the
      // write stays pending in memory and durable in storage, and the server
      // has still not received it when the walk pushes its diff.
      await drainPromiseJobs();
      markOpsSynced.mockClear();
      roots.reset();
      leaves.reset();
      sent.reset();
      acks.reset();
      second = makeClient(server.port, storage);
      await second.start();

      reconnectRoot = await roots.first((root) => root.mapName === OR_MAP, SIGNAL_TIMEOUT_MS);
      const leafForB = await leaves.first(
        (leaf) =>
          leaf.payload.mapName === OR_MAP && leaf.payload.entries.some((e) => e.key === KEY_B),
        SIGNAL_TIMEOUT_MS,
      );
      // The push for a leaf is sent inside the handler invocation that merged
      // it, so once that invocation settles the frame has left the client.
      if (leafForB) await leafForB.done;
      walkReachedKeyB = leafForB !== null;

      observed.droppedBatchCarriedLostOp = sent.items.some(carriesOp(observed.lostOpId, true));
      observed.triggerLeft = sent.items.some(
        (item) =>
          !item.dropped &&
          item.frame.type === 'ORMAP_PUSH_DIFF' &&
          item.frame.payload?.mapName === OR_MAP,
      );

      await observeAfterTrigger(
        observed,
        second,
        storage,
        markOpsSynced,
        server.port,
        MAP,
        K_NEXT,
        V_NEXT,
      );
    }, 180_000);

    afterAll(async () => {
      holdOpBatches = false;
      if (first) await first.close().catch(() => {});
      if (second) await second.close().catch(() => {});
      if (server) await server.cleanup().catch(() => {});
      // Removed only after the server has stopped: it holds the store and the WAL open.
      if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true });
    });

    /** A pass of the decisive test means nothing unless the scenario reached its state. */
    function expectScenarioReachedItsState(): void {
      expect(observed.lostOpId).toEqual(expect.any(String));
      expect(
        droppedBeforeReconnect
          ? 'the batch carrying the write was withheld'
          : 'no withheld OP_BATCH carried the write before the reconnect',
      ).toBe('the batch carrying the write was withheld');
      expect(
        observed.droppedBatchCarriedLostOp
          ? 'the reconnect flush carrying the write was withheld'
          : 'no withheld OP_BATCH carried the write after the reconnect',
      ).toBe('the reconnect flush carrying the write was withheld');

      expect(reconnectRoot).not.toBeNull();
      expect(
        reconnectRoot?.fullResync
          ? 'reconnect was routed to full resync — the incremental walk was not exercised'
          : 'incremental sync',
      ).toBe('incremental sync');
      expect(
        walkReachedKeyB
          ? 'walk reached key B'
          : 'no leaf response for key B — walk did not reach B',
      ).toBe('walk reached key B');
      expect(
        observed.triggerLeft ? 'ORMAP_PUSH_DIFF left the client' : 'ORMAP_PUSH_DIFF was not sent',
      ).toBe('ORMAP_PUSH_DIFF left the client');

      expect(observed.nextWrite).toBe('synced');
      expect(observed.serverValues.get(K_NEXT)).toBe(V_NEXT);
    }

    test('the server holds the write whose batch was lost in transit', () => {
      expect(observed.serverValues.get(K_LOST)).toBe(V_LOST);
      expectScenarioReachedItsState();
    });

    test('the scenario left the write pending and unapplied when the walk pushed its diff', () => {
      expectScenarioReachedItsState();
    });

    inStrayRegime('the push diff was answered with a stray acknowledgement', () => {
      expectStrayCoversThePendingOp(observed);
    });

    test('the write is still pending in memory after the push diff', () => {
      expect(observed.pendingCount).toBe(1);
    });

    test('the write is still pending in storage and nothing was durably marked synced', () => {
      expect(observed.pendingIdsInStorage).toContain(observed.lostOpId);
      expect(observed.markOpsSyncedArgs).toEqual([]);
    });

    // A bounded negative, as in L1: supportive only.
    inNoStrayRegime('no acknowledgement without a level reached the client', () => {
      expect(observed.acksWithoutLevel).toEqual([]);
    });
  });

  // ------------------------------------------------------------------------
  // W1: the wire shape itself, over the raw protocol, with no SDK logic.
  // ------------------------------------------------------------------------
  describe('W1: frames that are not operation batches, sent over the raw protocol', () => {
    const BARRIER_ID = 'w1-barrier';
    const MAP = 'stray_op_ack_w1';
    const OR_MAP = 'stray_op_ack_w1_or';

    interface Trigger {
      name: string;
      send: (client: TestClient) => void;
    }

    const emptyBatch = { type: 'OP_BATCH', payload: { ops: [] } };

    const triggers: Trigger[] = [
      {
        name: 'SEARCH_UNSUB',
        send: (c) => c.send({ type: 'SEARCH_UNSUB', payload: { subscriptionId: 'w1-search' } }),
      },
      {
        name: 'HYBRID_SEARCH_UNSUB',
        send: (c) =>
          c.send({ type: 'HYBRID_SEARCH_UNSUB', payload: { subscriptionId: 'w1-hybrid' } }),
      },
      {
        name: 'ORMAP_PUSH_DIFF',
        send: (c) =>
          c.send({
            type: 'ORMAP_PUSH_DIFF',
            payload: {
              mapName: OR_MAP,
              entries: [
                {
                  key: 'key-w1',
                  records: [createORRecord('w1', 'stray-op-ack-w1')],
                  tombstones: [],
                },
              ],
            },
          }),
      },
      { name: 'empty OP_BATCH', send: (c) => c.send(emptyBatch) },
      {
        name: 'BATCH holding an empty OP_BATCH',
        send: (c) => c.ws.send(encodeBatchFrame(packBatchData(serialize(emptyBatch)))),
      },
    ];

    let server: SpawnedServer | null = null;
    let raw: RawConnection | null = null;
    /** Regime 1: the acknowledgement that answered each trigger, or null if none came. */
    const answers = new Map<string, AckPayload | null>();
    let barrierAck: AckPayload | null = null;
    /** `lastId` of every OP_ACK received by the time the barrier write was acknowledged. */
    let ackIdsAtBarrier: string[] = [];

    beforeAll(async () => {
      server = await spawnRustServer();
      raw = await connectRaw(server.port, 'stray-op-ack-w1');
      const { client, frames } = raw;

      const claimed = new Set<InboundFrame>();
      for (const trigger of triggers) {
        trigger.send(client);
        if (!EXPECT_STRAY) continue;
        // One frame at a time, each answer awaited as an event before the next
        // frame is sent: that is what attributes an answer to its trigger.
        const answer = await frames.first(
          (frame) => frame.type === 'OP_ACK' && !claimed.has(frame),
          STRAY_TIMEOUT_MS,
        );
        if (answer) claimed.add(answer);
        answers.set(trigger.name, answer?.payload ?? null);
      }

      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: BARRIER_ID,
          mapName: MAP,
          opType: 'PUT',
          key: 'key-barrier',
          record: createLWWRecord({ barrier: true }, 'stray-op-ack-w1'),
        },
      });
      const barrier = await frames.first(isAckFor(BARRIER_ID), SIGNAL_TIMEOUT_MS);
      barrierAck = barrier?.payload ?? null;
      ackIdsAtBarrier = frames.items
        .filter((frame) => frame.type === 'OP_ACK')
        .map((frame) => String(frame.payload?.lastId));
    }, 180_000);

    afterAll(async () => {
      if (raw) raw.client.close();
      if (server) await server.cleanup().catch(() => {});
    });

    // LIMIT OF THIS LEG: it uses a later round trip (the barrier write) as its
    // barrier, and the server orders nothing between the answers to two frames.
    // A stray frame slower than the barrier's acknowledgement would be missed,
    // so this test CAN PASS FALSELY. It is a bounded negative, supportive only.
    inNoStrayRegime(
      'the only acknowledgement received is the one for the barrier write (bounded negative)',
      () => {
        expect(barrierAck?.lastId).toBe(BARRIER_ID);
        expect(ackIdsAtBarrier).toEqual([BARRIER_ID]);
      },
    );

    inStrayRegime('each of the five frames is answered with a stray acknowledgement', () => {
      const shapes = triggers.map((trigger) => {
        const answer = answers.get(trigger.name) ?? null;
        if (!answer) return { trigger: trigger.name, answer: 'none' };
        return {
          trigger: trigger.name,
          answer: isStray(answer) ? 'stray' : 'acknowledgement with a level or results',
          lastId: isNumeric(answer.lastId) ? 'numeric' : answer.lastId,
        };
      });

      expect(shapes).toEqual([
        { trigger: 'SEARCH_UNSUB', answer: 'stray', lastId: 'numeric' },
        { trigger: 'HYBRID_SEARCH_UNSUB', answer: 'stray', lastId: 'numeric' },
        { trigger: 'ORMAP_PUSH_DIFF', answer: 'stray', lastId: 'numeric' },
        { trigger: 'empty OP_BATCH', answer: 'stray', lastId: 'unknown' },
        { trigger: 'BATCH holding an empty OP_BATCH', answer: 'stray', lastId: 'numeric' },
      ]);
      expect(barrierAck?.lastId).toBe(BARRIER_ID);
      expect(isAbsent(barrierAck?.achievedLevel)).toBe(false);
    });
  });
});
