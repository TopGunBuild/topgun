/**
 * Integration: an LWW delete reaches the other online clients (TG-LWW-001).
 *
 * The server broadcasts an LWW REMOVE — a client's `remove` or the expiry
 * sweep's — with no record. A client subscribed to the map drops the key from
 * memory, from its listeners' view and from its storage, and its LWW Merkle
 * root then matches the server's, so its next sync walks nothing.
 *
 * Runs the release binary named by RUST_SERVER_BINARY. Waits poll with a
 * ceiling, never a fixed sleep.
 */

import * as path from 'path';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { LWWMap } from '@topgunbuild/core';

import { MerkleSyncHandler } from '../../packages/client/src/sync/MerkleSyncHandler';
import { SyncEngine } from '../../packages/client/src/SyncEngine';
import { WebSocketManager } from '../../packages/client/src/sync/WebSocketManager';
import { createTestToken, MemoryStorageAdapter } from './helpers';
import { spawnBinary, sleep, SpawnedBinary } from './helpers/two-binary';
import * as fs from 'fs';
import * as os from 'os';

const SWEEP_INTERVAL_MS = 1_000;
const TTL_MS = 1_500;
const SIGNAL_TIMEOUT_MS = 20_000;
/** TTL + sweep interval + one cycle, doubled. */
const SWEEP_CEILING_MS = 2 * (TTL_MS + 2 * SWEEP_INTERVAL_MS);
const STOP_TIMEOUT_MS = 40_000;
const REMOVED_LWW = 'topgun_expiry_removed_total{kind="lww_record"}';

async function until(
  what: string,
  probe: () => boolean | Promise<boolean>,
  ceilingMs = SIGNAL_TIMEOUT_MS,
): Promise<void> {
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    if (await probe()) return;
    if (Date.now() >= deadline) throw new Error(`${what}: not within ${ceilingMs} ms`);
    await sleep(50);
  }
}

async function metric(port: number, series: string): Promise<number> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${series} `));
  return line ? Number(line.slice(series.length + 1)) : 0;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the tests reach into the client's engine
const engineOf = (client: TopGunClient): any => (client as any).syncEngine;

describe('Integration: an LWW delete reaches other online clients', () => {
  let dir = '';
  let server: SpawnedBinary | null = null;
  let port = 0;
  const opened: TopGunClient[] = [];

  // Outbound frames and LWW root answers, attributed to the instance they ran on.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
  const sent: Array<{ on: unknown; msg: any }> = [];
  const roots: Array<{ on: unknown; mapName: string; done: Promise<void> }> = [];

  /** `before` runs ahead of `start`, so maps it opens are synced on connect. */
  const open = async (
    storage = new MemoryStorageAdapter(),
    before: (client: TopGunClient) => void = () => undefined,
  ): Promise<TopGunClient> => {
    const token = createTestToken('lww-remove-client', ['ADMIN']);
    const client = new TopGunClient({
      serverUrl: `ws://localhost:${port}/ws`,
      storage,
      auth: { getToken: () => Promise.resolve(token) },
      backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
    });
    opened.push(client);
    before(client);
    await client.start();
    await until('client connects', () => client.getConnectionState() === SyncState.CONNECTED);
    return client;
  };

  /** Opens the map and registers a live query, which routes the map's SERVER_EVENTs here. */
  const subscribed = async (client: TopGunClient, mapName: string) => {
    const map = client.getMap<string, string>(mapName);
    const query = client.query(mapName, {});
    query.subscribe(() => undefined);
    await query.whenSettled();
    return map;
  };

  /** Records every listener call of `map`, with whether its snapshot held `key`. */
  const listen = (map: LWWMap<string, string>, key: string) => {
    const calls: boolean[] = [];
    map.subscribe((entries) => calls.push(entries.some(([k]) => k === key)));
    return calls;
  };

  beforeAll(async () => {
    const originalSend = WebSocketManager.prototype.sendMessage;
    jest.spyOn(WebSocketManager.prototype, 'sendMessage').mockImplementation(function (
      this: WebSocketManager,
      message,
      key,
    ) {
      sent.push({ on: this, msg: message });
      return originalSend.call(this, message, key);
    });
    const originalRoot = MerkleSyncHandler.prototype.handleSyncRespRoot;
    jest.spyOn(MerkleSyncHandler.prototype, 'handleSyncRespRoot').mockImplementation(function (
      this: MerkleSyncHandler,
      payload,
    ) {
      const done = originalRoot.call(this, payload);
      roots.push({ on: this, mapName: payload.mapName, done });
      return done;
    });

    const binary = process.env.RUST_SERVER_BINARY;
    if (!binary)
      throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'lww-remove-event-'));
    server = await spawnBinary({
      binaryPath: binary,
      label: 'lww-remove-event',
      env: {
        STORAGE_BACKEND: 'redb',
        TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
        TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
        TOPGUN_WAL_DIR: path.join(dir, 'wal'),
        TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
        TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
        TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
      },
    });
    if (server.port === null) {
      throw new Error(`server did not start:\n${server.stderr()}\n${server.stdout()}`);
    }
    port = server.port;
  });

  afterAll(async () => {
    for (const client of opened) await client.close();
    await server?.stop('SIGTERM', STOP_TIMEOUT_MS);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
    jest.restoreAllMocks();
  });

  describe('a client delete', () => {
    const MAP = 'lww_remove_client';
    let b: TopGunClient;
    let bStorage: MemoryStorageAdapter;
    let bMap: LWWMap<string, string>;

    test('client delete reaches an online client', async () => {
      const a = await open();
      bStorage = new MemoryStorageAdapter();
      b = await open(bStorage);
      bMap = await subscribed(b, MAP);

      const aMap = a.getMap<string, string>(MAP);
      aMap.set('k', 'doomed');
      aMap.set('keep', 'kept');
      await until(
        'B holds both keys',
        () => bMap.get('k') === 'doomed' && bMap.get('keep') === 'kept',
      );
      await until(
        'B stored both keys',
        async () =>
          (await bStorage.get(`${MAP}:k`)) !== undefined &&
          (await bStorage.get(`${MAP}:keep`)) !== undefined,
      );

      const calls = listen(bMap, 'k');
      aMap.remove('k');
      await until('B drops k', () => bMap.getRecord('k') === undefined);
      await until(
        'B removed k from storage',
        async () => (await bStorage.get(`${MAP}:k`)) === undefined,
      );

      expect(bMap.get('k')).toBeUndefined();
      expect(bMap.getRecord('k')).toBeUndefined();
      expect(calls.length).toBeGreaterThanOrEqual(1);
      expect(calls[calls.length - 1]).toBe(false);
      expect(await bStorage.get(`${MAP}:k`)).toBeUndefined();
      expect(await bStorage.get(`${MAP}:keep`)).toBeDefined();
    });

    test('Merkle parity after the delete', async () => {
      let cMap!: LWWMap<string, string>;
      await open(undefined, (c) => {
        cMap = c.getMap<string, string>(MAP);
      });
      await until('C loads the map', () => cMap.get('keep') === 'kept');
      expect(bMap.getMerkleTree().getRootHash()).toBe(cMap.getMerkleTree().getRootHash());

      const engine = engineOf(b);
      const rootsBefore = roots.length;
      const sentBefore = sent.length;
      engine.webSocketManager.connectionProvider.forceReconnect();
      const answered = () =>
        roots
          .slice(rootsBefore)
          .filter((r) => r.on === engine.merkleSyncHandler && r.mapName === MAP);
      await until('B gets its LWW root answered again', () => answered().length >= 1);
      await Promise.all(answered().map((r) => r.done));

      const walks = sent
        .slice(sentBefore)
        .filter((f) => f.on === engine.webSocketManager)
        .filter((f) => f.msg?.type === 'MERKLE_REQ_BUCKET' && f.msg?.payload?.mapName === MAP);
      expect(walks).toEqual([]);
      expect(bMap.getMerkleTree().getRootHash()).toBe(cMap.getMerkleTree().getRootHash());
    });
  });

  test('expiry-sweep variant', async () => {
    const MAP = 'lww_remove_expiry';
    // Only the server's REMOVE may remove the record on B.
    const prune = jest
      .spyOn(
        SyncEngine.prototype as unknown as { pruneExpiredMaps(): Promise<void> },
        'pruneExpiredMaps',
      )
      .mockResolvedValue(undefined);
    try {
      const bStorage = new MemoryStorageAdapter();
      const b = await open(bStorage);
      const bMap = await subscribed(b, MAP);
      const calls = listen(bMap, 'k');

      const writer = await open();
      const wMap = writer.getMap<string, string>(MAP);
      const removedBefore = await metric(port, REMOVED_LWW);
      wMap.set('k', 'short-lived', TTL_MS);
      wMap.set('keep', 'kept');
      await until('B holds k', () => bMap.getRecord('k') !== undefined, TTL_MS);
      expect(bMap.getRecord('k')).toBeDefined();
      const callsBeforeSweep = calls.length;

      await until(
        'the sweep removed k',
        async () => (await metric(port, REMOVED_LWW)) >= removedBefore + 1,
        SWEEP_CEILING_MS,
      );
      await until('B drops k', () => bMap.getRecord('k') === undefined, SIGNAL_TIMEOUT_MS);
      await until(
        'B removed k from storage',
        async () => (await bStorage.get(`${MAP}:k`)) === undefined,
      );

      expect(calls.slice(callsBeforeSweep).some((held) => !held)).toBe(true);
      expect(await bStorage.get(`${MAP}:k`)).toBeUndefined();
      expect(bMap.get('keep')).toBe('kept');
    } finally {
      prune.mockRestore();
    }
  });
});
