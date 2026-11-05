/**
 * Integration: a record written with a TTL expires on the server, out of
 * process (TG-TTL-001, TG-TTL-002).
 *
 * Every case runs the release binary named by RUST_SERVER_BINARY with its own
 * data, and reads the server only through what a client can see: a query
 * snapshot, a Merkle walk (LWW and OR-Map), HTTP `/sync`, search, the journal,
 * `/metrics` and, for the eviction case, the server's own log.
 *
 * - filter: with the sweep never running, an expired record is already absent
 *   from every read path, so the hiding does not depend on the delete;
 * - sweep: with the sweep running, subscribers see an ordinary delete (LEAVE,
 *   journal DELETE with reason `expired`, SERVER_EVENT), a client connected
 *   before stops holding the record and one connecting after never gets it;
 * - pull: a client that catches up by Merkle sync receives `ttlMs`;
 * - restart and eviction: the TTL lives in the stored value, so a record read
 *   back from the store after a restart or an eviction still expires;
 * - convergence: after the sweep a client that held the records ends with the
 *   same LWW and OR-Map Merkle roots as a client that loads the map fresh, and
 *   its next sync walks nothing, whether it was connected during the sweep or
 *   came back after it.
 *
 * Waits poll with a ceiling, never a fixed sleep. The ceiling of a wait for
 * the sweep's effect is the TTL, plus the sweep interval, plus one cycle, plus
 * the client's one-second prune tick, doubled.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import { serialize, deserialize, LWWMap, ORMap } from '@topgunbuild/core';

import { MerkleSyncHandler } from '../../packages/client/src/sync/MerkleSyncHandler';
import { ORMapSyncHandler } from '../../packages/client/src/sync/ORMapSyncHandler';
import { WebSocketManager } from '../../packages/client/src/sync/WebSocketManager';
import { createTestToken, MemoryStorageAdapter, TestClient } from './helpers';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  plainLines,
  sleep,
  within,
  merkleLeaves,
  SpawnedBinary,
} from './helpers/two-binary';

const TTL_MS = 1_500;
const LONG_TTL_MS = 6_000;
/** Long enough to store the TTL keys, fill memory and see an eviction before expiry. */
const EVICT_TTL_MS = 10_000;
const SWEEP_INTERVAL_MS = 1_000;
/** A sweep interval no test outlives: the sweep never starts a cycle. */
const SWEEP_NEVER_MS = 86_400_000;
const CLIENT_PRUNE_TICK_MS = 1_000;
/** TTL + sweep interval + one cycle + client prune tick, doubled. */
const SWEEP_CEILING_MS =
  2 * (TTL_MS + SWEEP_INTERVAL_MS + SWEEP_INTERVAL_MS + CLIENT_PRUNE_TICK_MS);
/** Expiry by the read filter alone needs no sweep: one second past the TTL. */
const FILTER_CEILING_MS = TTL_MS + 1_000;
const PROBE_TIMEOUT_MS = 5_000;
const SIGNAL_TIMEOUT_MS = 20_000;
const STOP_TIMEOUT_MS = 40_000;

function serverBinary(): string {
  const binary = process.env.RUST_SERVER_BINARY;
  if (!binary) throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
  return binary;
}

function storeEnv(dir: string): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
    TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
  };
}

async function start(label: string, env: Record<string, string>): Promise<SpawnedBinary> {
  const server = await spawnBinary({ binaryPath: serverBinary(), label, env });
  if (server.port === null) {
    throw new Error(`${label} did not start:\n${server.stderr()}\n${server.stdout()}`);
  }
  return server;
}

function portOf(server: SpawnedBinary): number {
  return server.port as number;
}

/** Resolves once `probe` holds, or rejects naming `what` when the ceiling passes first. */
async function until(
  what: string,
  probe: () => boolean | Promise<boolean>,
  ceilingMs: number,
): Promise<void> {
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    if (await probe()) return;
    if (Date.now() >= deadline) throw new Error(`${what}: not within ${ceilingMs} ms`);
    await sleep(50);
  }
}

// ---------------------------------------------------------------------------
// Writes and reads over the wire
// ---------------------------------------------------------------------------

let opSeq = 0;

function stamp(nodeId: string): { millis: number; counter: number; nodeId: string } {
  return { millis: Date.now(), counter: 0, nodeId };
}

/** One acknowledged LWW PUT, with a TTL when `ttlMs` is given. */
async function put(
  client: TestClient,
  mapName: string,
  key: string,
  value: unknown,
  ttlMs?: number,
): Promise<void> {
  client.messages.length = 0;
  const record: Record<string, unknown> = { value, timestamp: stamp('ttl-writer') };
  if (ttlMs !== undefined) record.ttlMs = ttlMs;
  client.send({
    type: 'CLIENT_OP',
    payload: { id: `ttl-op-${opSeq++}`, mapName, opType: 'PUT', key, record },
  });
  const answer = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], PROBE_TIMEOUT_MS);
  if (answer?.type !== 'OP_ACK') {
    throw new Error(`PUT ${mapName}/${key} answered ${JSON.stringify(answer)}`);
  }
}

/** One acknowledged OR_ADD, with a TTL when `ttlMs` is given. */
async function orAdd(
  client: TestClient,
  mapName: string,
  key: string,
  value: unknown,
  ttlMs?: number,
): Promise<void> {
  client.messages.length = 0;
  const id = `ttl-op-${opSeq++}`;
  const orRecord: Record<string, unknown> = { value, timestamp: stamp('ttl-writer'), tag: id };
  if (ttlMs !== undefined) orRecord.ttlMs = ttlMs;
  client.send({
    type: 'CLIENT_OP',
    payload: { id, mapName, opType: 'OR_ADD', key, orRecord },
  });
  const answer = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], PROBE_TIMEOUT_MS);
  if (answer?.type !== 'OP_ACK') {
    throw new Error(`OR_ADD ${mapName}/${key} answered ${JSON.stringify(answer)}`);
  }
}

let readerSeq = 0;
const reader = (): string => `ttl-reader-${readerSeq++}`;

/** The values a one-shot HTTP `/sync` query returns for an LWW map (rows carry no key). */
async function httpValues(port: number, mapName: string): Promise<unknown[]> {
  const resp = await fetch(`http://127.0.0.1:${port}/sync`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/msgpack' },
    body: Buffer.from(
      serialize({
        clientId: reader(),
        clientHlc: stamp('ttl-http'),
        queries: [{ queryId: 'q', mapName, filter: null }],
      }),
    ),
  });
  if (resp.status !== 200) throw new Error(`HTTP /sync answered ${resp.status}`);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the response is untyped msgpack
  const body = deserialize<any>(new Uint8Array(await resp.arrayBuffer()));
  return body.queryResults?.[0]?.results ?? [];
}

/** The keys a full-text search over an LWW map finds. */
async function searchKeys(port: number, mapName: string, term: string): Promise<string[]> {
  const client = await connectWithin(port, reader());
  try {
    client.messages.length = 0;
    client.send({ type: 'SEARCH', payload: { requestId: 's', mapName, query: term } });
    const resp = await firstFrame(client, ['SEARCH_RESP'], PROBE_TIMEOUT_MS);
    if (!resp) throw new Error(`no SEARCH_RESP for ${mapName}`);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- search rows are untyped msgpack
    return resp.payload.results.map((r: any) => r.key);
  } finally {
    client.close();
  }
}

/** One series of `/metrics`, by its full name with labels; 0 when absent. */
async function metric(port: number, series: string): Promise<number> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${series} `));
  return line ? Number(line.slice(series.length + 1)) : 0;
}

const REMOVED_LWW = 'topgun_expiry_removed_total{kind="lww_record"}';
const REMOVED_OR = 'topgun_expiry_removed_total{kind="or_tag"}';

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- records are untyped msgpack
const expiredBy = (record: any, at: number): boolean =>
  record.ttlMs !== undefined && Number(record.timestamp.millis) + record.ttlMs < at;

/** What every read path a client has shows for the LWW key and the OR-Map key. */
interface Seen {
  snapshot: unknown;
  http: boolean;
  search: string[];
  /** The LWW leaf record, with whether its own stamp and TTL had run out when read. */
  lwwLeaf: { value: unknown; ttlMs: unknown; expired: boolean } | undefined;
  /** The OR-Map leaf's live records, likewise, and how many tombstones the key has. */
  orLeaf: { live: Array<{ value: unknown; ttlMs: unknown; expired: boolean }>; tombstones: number };
}

async function seen(
  port: number,
  lwwMap: string,
  lwwKey: string,
  orMap: string,
  orKey: string,
  marker: string,
): Promise<Seen> {
  const snapshot = await queryKey(port, lwwMap, lwwKey, reader());
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- HTTP rows are untyped msgpack
  const http = (await httpValues(port, lwwMap)).some((row: any) => row?.marker === marker);
  const search = await searchKeys(port, lwwMap, marker);
  const lww = (await merkleLeaves(port, lwwMap, false)).get(lwwKey);
  const or = (await merkleLeaves(port, orMap, true)).get(orKey);
  const at = Date.now();
  return {
    snapshot,
    http,
    search,
    lwwLeaf: lww && { value: lww.value, ttlMs: lww.ttlMs, expired: expiredBy(lww, at) },
    orLeaf: {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- records are untyped msgpack
      live: (or?.records ?? []).map((r: any) => ({
        value: r.value,
        ttlMs: r.ttlMs,
        expired: expiredBy(r, at),
      })),
      tombstones: or?.tombstones?.length ?? 0,
    },
  };
}

/** Both keys served before expiry, each with the TTL it was written with. */
function present(marker: string, ttlMs: number): Seen {
  return {
    snapshot: { marker },
    http: true,
    search: ['k'],
    lwwLeaf: { value: { marker }, ttlMs, expired: false },
    orLeaf: { live: [{ value: 'or-value', ttlMs, expired: false }], tombstones: 0 },
  };
}

/** Both keys deleted by the sweep: no LWW leaf, and the OR tag is a tombstone. */
const SWEPT: Seen = {
  snapshot: undefined,
  http: false,
  search: [],
  lwwLeaf: undefined,
  orLeaf: { live: [], tombstones: 1 },
};

const swept = (s: Seen): boolean => JSON.stringify(s) === JSON.stringify(SWEPT);

// ---------------------------------------------------------------------------
// Filter only: the sweep never runs
// ---------------------------------------------------------------------------

describe('Integration: an expired record is hidden by every read path before any sweep', () => {
  const LWW = 'ttl_filter_lww';
  const OR = 'ttl_filter_or';
  const MARKER = 'lanternfilter';
  let server: SpawnedBinary | null = null;
  let writtenAt = 0;
  let before: Seen | null = null;
  let beforeReadAt = 0;
  let after: Seen | null = null;
  let removed = { lww: -1, or: -1, cycles: -1 };

  beforeAll(async () => {
    server = await start('record-ttl filter', {
      TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_NEVER_MS),
    });
    const port = portOf(server);
    const writer = await connectWithin(port, 'ttl-filter-writer');
    writtenAt = Date.now();
    await put(writer, LWW, 'k', { marker: MARKER }, TTL_MS);
    await orAdd(writer, OR, 'k2', 'or-value', TTL_MS);
    writer.close();
    before = await seen(port, LWW, 'k', OR, 'k2', MARKER);
    beforeReadAt = Date.now();

    let last: Seen | null = null;
    await until(
      'every read path hides the expired keys',
      async () => swept((last = await seen(port, LWW, 'k', OR, 'k2', MARKER))),
      FILTER_CEILING_MS + (writtenAt + TTL_MS - Date.now()),
    ).catch(() => undefined);
    after = last;
    removed = {
      lww: await metric(port, REMOVED_LWW),
      or: await metric(port, REMOVED_OR),
      cycles: await metric(port, 'topgun_expiry_sweep_cycles_total'),
    };
  });

  afterAll(async () => {
    await server?.stop('SIGTERM', STOP_TIMEOUT_MS);
  });

  test('before expiry every read path returns both keys', () => {
    // Otherwise the presence below says nothing about the time before expiry.
    expect(beforeReadAt - writtenAt).toBeLessThan(TTL_MS);
    expect(before).toEqual(present(MARKER, TTL_MS));
  });

  test('after expiry the snapshot, HTTP /sync and search leave both keys out, and nothing was swept', () => {
    // The Merkle leaves still hold both records, unfiltered, with the TTL that
    // makes every client hide them by itself: the leaf must match the tree the
    // client compares, and nothing has been deleted yet.
    expect(after).toEqual({
      snapshot: undefined,
      http: false,
      search: [],
      lwwLeaf: { value: { marker: MARKER }, ttlMs: TTL_MS, expired: true },
      orLeaf: { live: [{ value: 'or-value', ttlMs: TTL_MS, expired: true }], tombstones: 0 },
    });
    expect(removed).toEqual({ lww: 0, or: 0, cycles: 0 });
  });
});

// ---------------------------------------------------------------------------
// The sweep: subscribers, a client connected before, a client connecting after
// ---------------------------------------------------------------------------

function makeClient(
  port: number,
  storage: MemoryStorageAdapter,
  token?: Promise<string>,
): TopGunClient {
  const ready = createTestToken('ttl-client', ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage,
    auth: { getToken: () => token ?? Promise.resolve(ready) },
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

async function connected(client: TopGunClient): Promise<void> {
  await client.start();
  await until(
    'client connects',
    () => client.getConnectionState() === SyncState.CONNECTED,
    SIGNAL_TIMEOUT_MS,
  );
}

describe('Integration: the sweep deletes an expired record the ordinary way', () => {
  const LWW = 'ttl_sweep_lww';
  const OR = 'ttl_sweep_or';
  const MARKER = 'lanternsweep';
  let server: SpawnedBinary | null = null;
  let subscriber: TestClient | null = null;
  let second: TopGunClient | null = null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- frames are untyped msgpack
  const frames = (): any[] => subscriber?.messages ?? [];
  let secondBefore: unknown[] = [];
  let secondAfter: unknown[] = [];
  let third: Seen | null = null;
  let removed = { lww: -1, or: -1 };

  beforeAll(async () => {
    server = await start('record-ttl sweep', {
      TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
    });
    const port = portOf(server);

    subscriber = await connectWithin(port, 'ttl-sweep-subscriber');
    subscriber.send({ type: 'QUERY_SUB', payload: { queryId: 'q-lww', mapName: LWW, query: {} } });
    subscriber.send({ type: 'JOURNAL_SUBSCRIBE', requestId: 'j-lww', mapName: LWW });
    subscriber.send({ type: 'JOURNAL_SUBSCRIBE', requestId: 'j-or', mapName: OR });
    await until(
      'the subscriber query is answered',
      () => frames().some((m) => m.type === 'QUERY_RESP'),
      PROBE_TIMEOUT_MS,
    );

    second = makeClient(port, new MemoryStorageAdapter());
    await connected(second);
    const lww = second.getMap<string, unknown>(LWW);
    const or = second.getORMap<string, string>(OR);
    // Live queries are what route SERVER_EVENTs of these maps to the client.
    const queries = [second.query(LWW, {}), second.query(OR, {})];
    queries.forEach((q) => q.subscribe(() => undefined));
    // Settled = the server answered the subscription, so it is registered.
    await within(Promise.all(queries.map((q) => q.whenSettled())), PROBE_TIMEOUT_MS, 'queries');

    const writer = await connectWithin(port, 'ttl-sweep-writer');
    await put(writer, LWW, 'k', { marker: MARKER }, TTL_MS);
    await orAdd(writer, OR, 'k2', 'or-value', TTL_MS);
    writer.close();
    await until(
      'the second client holds both records',
      () => lww.get('k') !== undefined && or.get('k2').length === 1,
      TTL_MS,
    ).catch(() => undefined);
    secondBefore = [lww.get('k'), or.get('k2')];

    await until(
      'the subscriber sees the LEAVE and both journal DELETEs',
      () =>
        frames().some((m) => m.type === 'QUERY_UPDATE' && m.payload?.changeType === 'LEAVE') &&
        frames().filter((m) => m.type === 'JOURNAL_EVENT' && m.event?.type === 'DELETE').length >=
          2,
      SWEEP_CEILING_MS,
    ).catch(() => undefined);
    await until(
      'the second client dropped both records',
      () => lww.getRecord('k') === undefined && (or.getRecordsMap('k2')?.size ?? 0) === 0,
      SWEEP_CEILING_MS,
    ).catch(() => undefined);
    secondAfter = [
      lww.get('k'),
      lww.getRecord('k'),
      or.get('k2'),
      or.getRecordsMap('k2')?.size ?? 0,
    ];

    third = await seen(port, LWW, 'k', OR, 'k2', MARKER);
    removed = { lww: await metric(port, REMOVED_LWW), or: await metric(port, REMOVED_OR) };
  });

  afterAll(async () => {
    subscriber?.close();
    await second?.close();
    await server?.stop('SIGTERM', STOP_TIMEOUT_MS);
  });

  test('a query subscriber receives LEAVE for the LWW key', () => {
    const leave = frames().find(
      (m) =>
        m.type === 'QUERY_UPDATE' &&
        m.payload?.queryId === 'q-lww' &&
        m.payload?.changeType === 'LEAVE',
    );
    expect(leave?.payload?.key).toBe('k');
  });

  test('a journal subscriber receives a DELETE with reason "expired" for each key', () => {
    const deletes = frames()
      .filter((m) => m.type === 'JOURNAL_EVENT' && m.event?.type === 'DELETE')
      .map((m) => ({ map: m.event.mapName, key: m.event.key, reason: m.event.metadata?.reason }));
    expect(deletes).toEqual(
      expect.arrayContaining([
        { map: LWW, key: 'k', reason: 'expired' },
        { map: OR, key: 'k2', reason: 'expired' },
      ]),
    );
  });

  test('the sweep removed one LWW record and one OR tag', () => {
    expect(removed).toEqual({ lww: 1, or: 1 });
  });

  test('a client connected before sees both keys, then holds neither', () => {
    expect(secondBefore).toEqual([{ marker: MARKER }, ['or-value']]);
    expect(secondAfter).toEqual([undefined, undefined, [], 0]);
  });

  test('a client connecting after expiry never receives either key as live', () => {
    expect(third).toEqual(SWEPT);
  });
});

// ---------------------------------------------------------------------------
// TODO-721: a client catching up by Merkle sync receives the TTL
// ---------------------------------------------------------------------------

describe('Integration: a client that syncs after the write receives ttlMs', () => {
  const LWW = 'ttl_pull_lww';
  const OR = 'ttl_pull_or';
  const PULL_TTL_MS = 600_000;
  let server: SpawnedBinary | null = null;
  let client: TopGunClient | null = null;
  let held: unknown[] = [];

  beforeAll(async () => {
    server = await start('record-ttl pull', {});
    const port = portOf(server);
    const writer = await connectWithin(port, 'ttl-pull-writer');
    await put(writer, LWW, 'k', 'lww-value', PULL_TTL_MS);
    await orAdd(writer, OR, 'k2', 'or-value', PULL_TTL_MS);
    writer.close();

    client = makeClient(port, new MemoryStorageAdapter());
    const lww = client.getMap<string, string>(LWW);
    const or = client.getORMap<string, string>(OR);
    await connected(client);
    await until(
      'the walk delivered both records',
      () => lww.getRecord('k') !== undefined && or.getRecords('k2').length === 1,
      SIGNAL_TIMEOUT_MS,
    ).catch(() => undefined);
    held = [lww.getRecord('k')?.ttlMs, or.getRecords('k2')[0]?.ttlMs];
  });

  afterAll(async () => {
    await client?.close();
    await server?.stop('SIGTERM', STOP_TIMEOUT_MS);
  });

  test('the LWW record and the OR-Map record carry the TTL they were written with', () => {
    expect(held).toEqual([PULL_TTL_MS, PULL_TTL_MS]);
  });
});

// ---------------------------------------------------------------------------
// Restart and eviction: the TTL is read back from the store
// ---------------------------------------------------------------------------

describe('Integration: a TTL survives a restart', () => {
  const LWW = 'ttl_restart_lww';
  const OR = 'ttl_restart_or';
  const MARKER = 'lanternrestart';
  const RESTART_AT_MS = 2_000;
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];
  let writtenAt = 0;
  let restartMs = 0;
  let firstStop: unknown = null;
  let afterRestart: Seen | null = null;
  let afterRestartAt = 0;
  let afterExpiry: Seen | null = null;
  let removed = { lww: -1, or: -1 };

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-record-ttl-restart-'));
    const env = { ...storeEnv(dir), TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS) };
    const first = await start('record-ttl restart, first run', env);
    running.push(first);
    const writer = await connectWithin(portOf(first), 'ttl-restart-writer');
    writtenAt = Date.now();
    await put(writer, LWW, 'k', { marker: MARKER }, LONG_TTL_MS);
    await orAdd(writer, OR, 'k2', 'or-value', LONG_TTL_MS);
    writer.close();

    await sleep(Math.max(0, writtenAt + RESTART_AT_MS - Date.now()));
    const stopping = Date.now();
    firstStop = await first.stop('SIGTERM', STOP_TIMEOUT_MS);
    const second = await start('record-ttl restart, second run', env);
    running.push(second);
    restartMs = Date.now() - stopping;
    const port = portOf(second);

    afterRestart = await seen(port, LWW, 'k', OR, 'k2', MARKER);
    afterRestartAt = Date.now();

    let last: Seen | null = null;
    await until(
      'the restarted server hides and sweeps both keys',
      async () =>
        swept((last = await seen(port, LWW, 'k', OR, 'k2', MARKER))) &&
        (await metric(port, REMOVED_LWW)) >= 1 &&
        (await metric(port, REMOVED_OR)) >= 1,
      Math.max(0, writtenAt + LONG_TTL_MS - Date.now()) + SWEEP_CEILING_MS,
    ).catch(() => undefined);
    afterExpiry = last;
    removed = { lww: await metric(port, REMOVED_LWW), or: await metric(port, REMOVED_OR) };
  });

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('the first run stops cleanly and the restart leaves the keys time to be seen', () => {
    expect(firstStop).toEqual({ code: 0, signal: null });
    // The restart must end before expiry, or "visible after the restart"
    // below is never checked and the case proves only "absent after".
    expect(afterRestartAt).toBeLessThan(writtenAt + LONG_TTL_MS);
    process.stderr.write(`[record-ttl] stop and start took ${restartMs} ms\n`);
  });

  test('after the restart, before expiry, both keys are served from the store', () => {
    expect(afterRestart).toEqual(present(MARKER, LONG_TTL_MS));
  });

  test('after expiry the restarted server hides both keys and its sweep removes them', () => {
    expect(afterExpiry).toEqual(SWEPT);
    expect(removed).toEqual({ lww: 1, or: 1 });
  });
});

describe('Integration: a TTL survives an eviction', () => {
  const LWW = 'ttl_evict_lww';
  const OR = 'ttl_evict_or';
  const FILL = 'ttl_evict_fill';
  const MARKER = 'lanternevict';
  /** Above the 1 MiB ceiling with room to spare; about 2 MB of values. */
  const FILLERS = 120;
  const FILLER_BYTES = 16_384;
  const EVICTION_LINE = /eviction_orchestrator: evicted .*evicted=[1-9]/;
  const SHORT_OF_TARGET = /Eviction fell short of target/;
  let dir: string | null = null;
  let server: SpawnedBinary | null = null;
  let writtenAt = 0;
  let evictedAt = 0;
  let evictionLines: string[] = [];
  let shortForTtlMaps: string[] = [];
  let beforeExpiry: Seen | null = null;
  let beforeExpiryAt = 0;
  let afterExpiry: Seen | null = null;
  let removed = { lww: -1, or: -1 };

  beforeAll(async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-record-ttl-evict-'));
    server = await start('record-ttl eviction', {
      ...storeEnv(dir),
      TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
      TOPGUN_MAX_RAM_MB: '1',
      TOPGUN_EVICTION_INTERVAL_MS: '100',
      TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '100',
    });
    const port = portOf(server);
    const writer = await connectWithin(port, 'ttl-evict-writer');

    // The orchestrator's log line only sums what a tick evicted, so the layout
    // makes that sum enough. Each TTL key is written first, alone in its map,
    // and stored (the write-behind buffer drains) before any filler: from then
    // on it is clean and is the oldest record. The fillers go to a third map
    // without a TTL and push the node over its 1 MiB ceiling. A tick above the
    // high water mark gives every store that holds anything a target of at
    // least one record, so a tick that evicts takes the TTL key along, unless
    // it reports its store as short of target, which is checked as well.
    writtenAt = Date.now();
    await put(writer, LWW, 'k', { marker: MARKER }, EVICT_TTL_MS);
    await orAdd(writer, OR, 'k2', 'or-value', EVICT_TTL_MS);
    await until(
      'the TTL keys are stored',
      async () => (await metric(port, 'topgun_write_behind_buffered_entries')) === 0,
      EVICT_TTL_MS,
    );
    const stored = server.stdout().length;
    const pad = 'x'.repeat(FILLER_BYTES);
    for (let i = 0; i < FILLERS; i++) await put(writer, FILL, `f${i}`, { pad });
    writer.close();

    await until(
      'the eviction orchestrator evicted records',
      () => plainLines(server!.stdout().slice(stored)).some((l) => EVICTION_LINE.test(l)),
      EVICT_TTL_MS,
    ).catch(() => undefined);
    evictedAt = Date.now();
    const lines = plainLines(server.stdout().slice(stored));
    evictionLines = lines.filter((l) => EVICTION_LINE.test(l));
    shortForTtlMaps = lines.filter(
      (l) => SHORT_OF_TARGET.test(l) && (l.includes(`map=${LWW}`) || l.includes(`map=${OR}`)),
    );

    beforeExpiry = await seen(port, LWW, 'k', OR, 'k2', MARKER);
    beforeExpiryAt = Date.now();

    let last: Seen | null = null;
    await until(
      'both keys are hidden and swept',
      async () =>
        swept((last = await seen(port, LWW, 'k', OR, 'k2', MARKER))) &&
        (await metric(port, REMOVED_LWW)) >= 1 &&
        (await metric(port, REMOVED_OR)) >= 1,
      Math.max(0, writtenAt + EVICT_TTL_MS - Date.now()) + SWEEP_CEILING_MS,
    ).catch(() => undefined);
    afterExpiry = last;
    removed = { lww: await metric(port, REMOVED_LWW), or: await metric(port, REMOVED_OR) };
  });

  afterAll(async () => {
    await server?.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('the server evicted records after the TTL keys were stored and before they expired', () => {
    expect(evictionLines.length).toBeGreaterThan(0);
    expect(shortForTtlMaps).toEqual([]);
    expect(evictedAt).toBeLessThan(writtenAt + EVICT_TTL_MS);
  });

  test('after the eviction, before expiry, the reads reach the stored rows', () => {
    expect(beforeExpiryAt).toBeLessThan(writtenAt + EVICT_TTL_MS);
    expect(beforeExpiry).toEqual(present(MARKER, EVICT_TTL_MS));
  });

  test('after expiry both keys are hidden and the sweep removes them', () => {
    expect(afterExpiry).toEqual(SWEPT);
    expect(removed).toEqual({ lww: 1, or: 1 });
  });
});

// ---------------------------------------------------------------------------
// Convergence: after the sweep, a client that held the records ends with the
// roots of a client that loads the map fresh, and its next sync walks nothing
// ---------------------------------------------------------------------------

describe('Integration: a client converges with the server after the sweep', () => {
  const WALK_FRAMES = ['MERKLE_REQ_BUCKET', 'ORMAP_MERKLE_REQ_BUCKET'];

  // Calls are attributed to a client by the instance they ran on, so a handler
  // of a client already closed is never counted for the next one.
  const owners = new WeakMap<object, string>();
  const ownerOf = (instance: unknown): string =>
    (typeof instance === 'object' && instance !== null && owners.get(instance)) || 'unowned';
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- outbound frames have no single static shape
  const sent: Array<{ owner: string; msg: any }> = [];
  const roots: Array<{ owner: string; mapName: string; done: Promise<void> }> = [];
  const opened: TopGunClient[] = [];

  function engineOf(client: TopGunClient): Record<string, object> {
    return (client as unknown as { syncEngine: Record<string, object> }).syncEngine;
  }

  function observed(
    owner: string,
    port: number,
    storage: MemoryStorageAdapter,
    token?: Promise<string>,
  ): TopGunClient {
    const client = makeClient(port, storage, token);
    const engine = engineOf(client);
    owners.set(engine, owner);
    for (const part of ['webSocketManager', 'merkleSyncHandler', 'orMapSyncHandler']) {
      if (!engine[part]) throw new Error(`client has no ${part} to observe`);
      owners.set(engine[part], owner);
    }
    opened.push(client);
    return client;
  }

  beforeAll(() => {
    const originalSend = WebSocketManager.prototype.sendMessage;
    jest.spyOn(WebSocketManager.prototype, 'sendMessage').mockImplementation(function (
      this: WebSocketManager,
      message,
      key,
    ) {
      sent.push({ owner: ownerOf(this), msg: message });
      return originalSend.call(this, message, key);
    });
    const originalLwwRoot = MerkleSyncHandler.prototype.handleSyncRespRoot;
    jest.spyOn(MerkleSyncHandler.prototype, 'handleSyncRespRoot').mockImplementation(function (
      this: MerkleSyncHandler,
      payload,
    ) {
      const done = originalLwwRoot.call(this, payload);
      roots.push({ owner: ownerOf(this), mapName: payload.mapName, done });
      return done;
    });
    const originalOrRoot = ORMapSyncHandler.prototype.handleORMapSyncRespRoot;
    jest.spyOn(ORMapSyncHandler.prototype, 'handleORMapSyncRespRoot').mockImplementation(function (
      this: ORMapSyncHandler,
      payload,
    ) {
      const done = originalOrRoot.call(this, payload);
      roots.push({ owner: ownerOf(this), mapName: payload.mapName, done });
      return done;
    });
  });

  afterAll(async () => {
    for (const client of opened) await client.close();
    jest.restoreAllMocks();
  });

  /**
   * Keeps the server's tombstone protection dark for the whole run. Once it is
   * active, the server answers every OR-Map sync init of a client that claims
   * no confirmed epoch with a full-snapshot resync (TG-MRK-002), which walks
   * whatever the roots are; that is the reclamation protocol, not expiry, and
   * would hide what this case asserts. Protection activates when the durable
   * epoch watermark leaves 0, and the watermark stops at the epoch of the
   * first remove whose write is still unflushed. So the first remove the
   * server sees is on a map nothing touches again, and the write-behind flush
   * interval is far above the run's duration: that remove never flushes, and
   * the watermark stays at 0 (the same pin as merkle-comparability.test.ts).
   */
  async function pinProtectionDark(port: number, variant: string): Promise<void> {
    const pin = await connectWithin(port, `ttl-conv-${variant}-pin`);
    try {
      await orAdd(pin, `ttl_conv_${variant}_pin`, 'pin', 'pinned');
      const tag = [
        ...(await merkleLeaves(port, `ttl_conv_${variant}_pin`, true)).get('pin').records,
      ][0].tag;
      pin.messages.length = 0;
      pin.send({
        type: 'CLIENT_OP',
        payload: {
          id: `ttl-conv-${variant}-pin-remove`,
          mapName: `ttl_conv_${variant}_pin`,
          opType: 'OR_REMOVE',
          key: 'pin',
          orTag: tag,
        },
      });
      const answer = await firstFrame(pin, ['OP_ACK', 'OP_REJECTED', 'ERROR'], PROBE_TIMEOUT_MS);
      if (answer?.type !== 'OP_ACK')
        throw new Error(`pin remove answered ${JSON.stringify(answer)}`);
    } finally {
      pin.close();
    }
  }

  interface Outcome {
    /** Empty when every precondition of the run was reached, else what was not. */
    staging: string;
    ttlTag: string;
    heldRoots: { lww: number; or: number };
    freshRoots: { lww: number; or: number };
    /** Walk frames the client that held the records sent before the comparison. */
    heldWalks: string[];
    /** Walk frames of the further sync, and whether its roots were answered. */
    furtherWalks: string[];
    furtherRootsAnswered: number;
    /** Pushes from the held client: the expired tag as a tombstone, and as a live record. */
    pushedAsTombstone: number;
    pushedAsLive: number;
    /** The server after everything, read by a third connection. */
    serverOr: { live: number; tombstones: string[] };
    serverLwwHasKey: boolean;
    serverHttpHasKey: boolean;
  }

  async function run(variant: 'connected' | 'reconnected'): Promise<Outcome> {
    const LWW = `ttl_conv_${variant}_lww`;
    const OR = `ttl_conv_${variant}_or`;
    const MARKER = `lanternconv${variant}`;
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-record-ttl-conv-${variant}-`));
    const server = await start(`record-ttl convergence, ${variant}`, {
      ...storeEnv(dir),
      TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
      TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: '600000',
      TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '600000',
    });
    const port = portOf(server);
    await pinProtectionDark(port, variant);
    const held = `${variant}:held`;
    const storage = new MemoryStorageAdapter();
    const outcome: Outcome = {
      staging: '',
      ttlTag: '',
      heldRoots: { lww: -1, or: -1 },
      freshRoots: { lww: -2, or: -2 },
      heldWalks: [],
      furtherWalks: [],
      furtherRootsAnswered: 0,
      pushedAsTombstone: -1,
      pushedAsLive: -1,
      serverOr: { live: -1, tombstones: [] },
      serverLwwHasKey: true,
      serverHttpHasKey: true,
    };

    /** A client over `storage` that authenticates only once both maps are restored. */
    async function reload(
      owner: string,
      restored: (lww: LWWMap<string, unknown>, or: ORMap<string, string>) => boolean,
    ): Promise<TopGunClient> {
      let release: () => void = () => undefined;
      const token = new Promise<string>((resolve) => {
        release = () => resolve(createTestToken('ttl-client', ['ADMIN']));
      });
      const client = observed(owner, port, storage, token);
      await client.start();
      const lww = client.getMap<string, unknown>(LWW);
      const or = client.getORMap<string, string>(OR);
      await until(
        `${owner}: both maps restored from storage`,
        () => restored(lww, or),
        SIGNAL_TIMEOUT_MS,
      );
      release();
      await until(
        `${owner}: connected`,
        () => client.getConnectionState() === SyncState.CONNECTED,
        SIGNAL_TIMEOUT_MS,
      );
      return client;
    }

    const holdsSweptState = (c: TopGunClient): boolean =>
      c.getMap(LWW).getRecord('ttl') === undefined &&
      c.getMap(LWW).getRecord('keep') !== undefined &&
      (c.getORMap(OR).getRecordsMap('ttl')?.size ?? 0) === 0 &&
      c.getORMap(OR).getKeyTombstones('ttl').has(outcome.ttlTag) &&
      (c.getORMap(OR).getRecordsMap('keep')?.size ?? 0) === 1;

    const walksOf = (owner: string): string[] =>
      sent
        .filter((f) => f.owner === owner && WALK_FRAMES.includes(f.msg?.type))
        .filter((f) => [LWW, OR].includes(f.msg?.payload?.mapName))
        .map((f) => f.msg.type as string);

    try {
      // --- B holds a TTL record and a lasting one in each map, from live events. ---
      let holder = observed(held, port, storage);
      const lww = holder.getMap<string, unknown>(LWW);
      const or = holder.getORMap<string, string>(OR);
      await connected(holder);
      const queries = [holder.query(LWW, {}), holder.query(OR, {})];
      queries.forEach((q) => q.subscribe(() => undefined));
      await within(Promise.all(queries.map((q) => q.whenSettled())), PROBE_TIMEOUT_MS, 'queries');

      const writer = await connectWithin(port, `ttl-conv-${variant}-writer`);
      await put(writer, LWW, 'keep', { marker: 'keep' });
      await put(writer, LWW, 'ttl', { marker: MARKER }, TTL_MS);
      await orAdd(writer, OR, 'keep', 'kept');
      await orAdd(writer, OR, 'ttl', 'expiring', TTL_MS);
      writer.close();

      await until(
        'the client holds all four records before they expire',
        () =>
          lww.getRecord('keep') !== undefined &&
          lww.getRecord('ttl') !== undefined &&
          (or.getRecordsMap('keep')?.size ?? 0) === 1 &&
          (or.getRecordsMap('ttl')?.size ?? 0) === 1,
        TTL_MS,
      );
      outcome.ttlTag = [...(or.getRecordsMap('ttl')?.keys() ?? [])][0];

      if (variant === 'reconnected') {
        await holder.close();
      }

      // --- The server expires and sweeps both TTL records. ---
      await until(
        'the server swept both records',
        async () => (await metric(port, REMOVED_LWW)) >= 1 && (await metric(port, REMOVED_OR)) >= 1,
        SWEEP_CEILING_MS,
      );

      if (variant === 'reconnected') {
        // Back after the sweep: the walk of the reconnect brings the tombstone.
        holder = await reload(
          `${variant}:back`,
          (l, o) => l.getRecord('keep') !== undefined && (o.getRecordsMap('ttl')?.size ?? 0) === 1,
        );
      }
      // The LWW record leaves through the client's own prune tick, the OR tag
      // through the server's tombstone (an event, or the walk of the reconnect).
      await until(
        'the held client drops both records',
        () => holdsSweptState(holder),
        SWEEP_CEILING_MS,
      );
      outcome.heldWalks = walksOf(variant === 'reconnected' ? `${variant}:back` : held);

      // --- C loads the maps fresh. ---
      const fresh = observed(`${variant}:fresh`, port, new MemoryStorageAdapter());
      fresh.getMap(LWW);
      fresh.getORMap(OR);
      await connected(fresh);
      await until(
        'the fresh client loaded both maps',
        () => holdsSweptState(fresh),
        SIGNAL_TIMEOUT_MS,
      );

      outcome.heldRoots = {
        lww: holder.getMap(LWW).getMerkleTree().getRootHash(),
        or: holder.getORMap(OR).getMerkleTree().getRootHash(),
      };
      outcome.freshRoots = {
        lww: fresh.getMap(LWW).getMerkleTree().getRootHash(),
        or: fresh.getORMap(OR).getMerkleTree().getRootHash(),
      };

      // --- B syncs once more: the same client reconnects. ---
      // The same instance, because the confirmed-apply cursor it reports on
      // the OR-Map init lives in memory: a new instance over the same storage
      // reports none, and once the server holds a tombstone it answers such a
      // client with a full-snapshot resync by design (TG-MRK-002), whatever
      // the client's root.
      const owner = ownerOf(engineOf(holder));
      const rootsBefore = roots.length;
      const sentBefore = sent.length;
      (
        engineOf(holder).webSocketManager as unknown as {
          connectionProvider: { forceReconnect(): void };
        }
      ).connectionProvider.forceReconnect();
      const further = (): typeof roots =>
        roots.slice(rootsBefore).filter((r) => r.owner === owner && [LWW, OR].includes(r.mapName));
      await until(
        'the further sync answered both roots',
        () => further().length >= 2,
        SIGNAL_TIMEOUT_MS,
      );
      // Settled = the root decision was taken and anything it sends was sent.
      await Promise.all(further().map((r) => r.done));
      outcome.furtherRootsAnswered = further().length;
      outcome.furtherWalks = sent
        .slice(sentBefore)
        .filter((f) => f.owner === owner && WALK_FRAMES.includes(f.msg?.type))
        .filter((f) => [LWW, OR].includes(f.msg?.payload?.mapName))
        .map((f) => f.msg.type as string);

      // --- What B ever pushed for the expired tag. ---
      const pushes = sent
        .filter((f) => f.owner.startsWith(`${variant}:`) && f.owner !== `${variant}:fresh`)
        .filter((f) => f.msg?.type === 'ORMAP_PUSH_DIFF' && f.msg?.payload?.mapName === OR)
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- push entries are untyped
        .flatMap((f) => f.msg.payload.entries as any[]);
      outcome.pushedAsTombstone = pushes.filter((e) =>
        (e.tombstones ?? []).includes(outcome.ttlTag),
      ).length;
      outcome.pushedAsLive = pushes.filter((e) =>
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- push records are untyped
        (e.records ?? []).some((r: any) => r.tag === outcome.ttlTag),
      ).length;

      // --- The server, read by a third connection and over HTTP. ---
      const orState = (await merkleLeaves(port, OR, true)).get('ttl');
      outcome.serverOr = {
        live: orState?.records?.length ?? 0,
        tombstones: orState?.tombstones ?? [],
      };
      outcome.serverLwwHasKey = (await merkleLeaves(port, LWW, false)).has('ttl');
      outcome.serverHttpHasKey = (await httpValues(port, LWW)).some(
        // eslint-disable-next-line @typescript-eslint/no-explicit-any -- HTTP rows are untyped msgpack
        (row: any) => row?.marker === MARKER,
      );
    } catch (err) {
      outcome.staging = err instanceof Error ? err.message : String(err);
    } finally {
      for (const client of opened.splice(0)) await client.close();
      await server.stop('SIGKILL', 5_000);
      fs.rmSync(dir, { recursive: true, force: true });
    }
    process.stderr.write(`[record-ttl] convergence ${variant}: ${JSON.stringify(outcome)}\n`);
    return outcome;
  }

  for (const variant of ['connected', 'reconnected'] as const) {
    describe(`the client was ${variant === 'connected' ? 'connected during' : 'away during'} the sweep`, () => {
      let o: Outcome;

      beforeAll(async () => {
        o = await run(variant);
      }, 120_000);

      test('every precondition of the run was reached', () => {
        expect(o.staging).toBe('');
      });

      test(`the OR tombstone reached the client ${variant === 'connected' ? 'by an event, with no walk' : 'by the walk of its reconnect'}`, () => {
        if (variant === 'connected') {
          expect(o.heldWalks).toEqual([]);
        } else {
          expect(o.heldWalks).toContain('ORMAP_MERKLE_REQ_BUCKET');
        }
      });

      test('its LWW and OR-Map roots equal those of a client that loads the maps fresh', () => {
        expect(o.heldRoots).toEqual(o.freshRoots);
        expect(o.heldRoots.lww).not.toBe(0);
        expect(o.heldRoots.or).not.toBe(0);
      });

      test('its further sync compares both roots and walks nothing', () => {
        expect(o.furtherRootsAnswered).toBe(2);
        expect(o.furtherWalks).toEqual([]);
      });

      test('it never pushes the expired tag as a tombstone', () => {
        expect(o.pushedAsTombstone).toBe(0);
        process.stderr.write(
          `[record-ttl] ${variant}: pushes re-sending the expired tag as live: ${o.pushedAsLive}\n`,
        );
      });

      test('the server holds the tombstone and no live entry for the tag, and no LWW record', () => {
        expect(o.serverOr).toEqual({ live: 0, tombstones: [o.ttlTag] });
        expect(o.serverLwwHasKey).toBe(false);
        expect(o.serverHttpHasKey).toBe(false);
      });
    });
  }
});
