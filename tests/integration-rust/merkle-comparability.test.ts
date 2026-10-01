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
