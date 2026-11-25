/**
 * Integration: a live subscribe's snapshot carries the writes made during its
 * scan, and a write burst that outgrows the snapshot buffer ends in an
 * overflow the client recovers from, on a real server (TG-LIVE-001).
 *
 * The subscribe is held between the end of its scan and the seeding of its
 * window by a server built with the `fault-injection` cargo feature: while the
 * file TOPGUN_FAULT_QUERY_DRAIN_HOLD_FILE names exists, a live subscribe waits
 * there with its snapshot buffer active, and logs one WARN with its query_id.
 * The binary is given by path (FAULT_SERVER_BINARY) together with the commit it
 * was built from (FAULT_SERVER_COMMIT). With either missing the test fails; it
 * never skips and never falls back to another binary.
 *
 * Two clients: S, the TopGun client, subscribes; W, a protocol-level
 * connection of its own, writes. W seeds map `m` with 50 rows first. W sends
 * each batch as one OP_BATCH and waits for its OP_ACK, so the test decides
 * exactly which writes reach the server, and S as SERVER_EVENTs, before each
 * step.
 *
 * - Case B (fold). S subscribes `qB` (no predicate, limit or sort, so every key
 *   of `m` is on its page) and is held. W updates `s-0` and writes 10 new keys.
 *   S's first frame for `qB` must be a QUERY_RESP that holds all eleven writes,
 *   with no QUERY_UPDATE of `qB` before it: the writes were folded into the
 *   snapshot instead of being sent ahead of it.
 * - Case C (overflow). S subscribes `qC` and is held. W writes 4097 new keys,
 *   one more than the buffer holds, in batches of at most 100; before the next
 *   batch the test waits for the batch's acknowledgement and for S to have
 *   received the batch's SERVER_EVENTs, so S's outbound channel never holds a
 *   burst. S must then receive the snapshot, then QUERY_SNAPSHOT_OVERFLOW, and
 *   resubscribe once; the resubscribed rows must equal every key of `m`, the
 *   row count must never fall, and S's connection must never change state.
 *
 * Inbound frames are recorded through the engine's generic message listener
 * (`SyncEngine.on('message')`, which sees every frame a BATCH envelope carries
 * as well), outbound frames by wrapping S's `WebSocketManager.sendMessage`.
 *
 * Environment:
 *
 * - LIVENESS_OUT_DIR — where the run writes its notes and the server's stdout;
 *                      unset, nothing is written;
 * - LIVENESS_RUN     — label of the run in those file names.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { QueryHandle, StateChangeEvent } from '@topgunbuild/client';

import { createTestToken, MemoryStorageAdapter } from './helpers';
import { SEAM_BOOT_TEXT, requireFaultBinary } from './helpers/fault-binary';
import { TestClient } from './helpers/test-client';
import {
  connectWithin,
  firstFrame,
  logField,
  plainLines,
  sleep,
  spawnBinary,
  SpawnedBinary,
} from './helpers/two-binary';

const MAP = 'snapshot_overflow_m';
const SEED_ROWS = 50;
const FOLD_KEYS = 10;
/** One more than the server's snapshot buffer holds (`SNAPSHOT_BUFFER_CAPACITY`). */
const OVERFLOW_KEYS = 4_097;
const BATCH = 100;

const HOLD_ENV = 'TOPGUN_FAULT_QUERY_DRAIN_HOLD_FILE';
/** The boot line of a build that has the query-hold seam and was given a sentinel path. */
const HOLD_BOOT_TEXT = 'live subscribes are held while the query-hold sentinel file exists';
/** The WARN of one held subscribe; the boot line does not contain it. */
const HOLD_TEXT = 'a live subscribe is held while the sentinel file exists';
const SLOW_CONSUMER_TEXT = 'slow consumer exceeded broadcast drop threshold';
const LIVE_LOSS_TEXT = 'a live event owed to a subscriber was lost';
const OVERFLOW_CODE = 'QUERY_SNAPSHOT_OVERFLOW';
const OVERFLOW_SERIES = 'topgun_query_snapshot_overflow_total{reason="buffer"}';

/**
 * The longest a hold may last: half of the client's 15 s heartbeat timeout
 * (its 5 s interval is the client default too), so a held global worker can
 * never make a connection look dead.
 */
const MAX_HOLD_MS = 7_500;
const CONNECT_WAIT_MS = 20_000;
const STEP_WAIT_MS = 10_000;
/** Covers the client's first resubscribe after an overflow (250 ms with jitter) and a 4157-row scan. */
const RESUBSCRIBE_WAIT_MS = 30_000;

interface Inbound {
  type: string;
  queryId?: string;
  code?: string;
  /** The keys of a QUERY_RESP, and the values by key, for the frames the cases read. */
  results?: Map<string, unknown>;
  /** SERVER_EVENTs this frame carries for MAP. */
  events: number;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test reaches into the client's engine
const engineOf = (client: TopGunClient): any => (client as any).syncEngine;

async function until(what: string, probe: () => boolean, ceilingMs: number): Promise<void> {
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    if (probe()) return;
    if (Date.now() >= deadline) throw new Error(`${what}: not within ${ceilingMs} ms`);
    await sleep(10);
  }
}

async function metricLine(port: number, series: string): Promise<string | undefined> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  return text.split('\n').find((line) => line.startsWith(`${series} `));
}

describe('Integration: a live subscribe folds the writes of its scan and overflows in order', () => {
  let dir = '';
  let server: SpawnedBinary | null = null;
  const clients: TopGunClient[] = [];
  let writer: TestClient | null = null;
  const notes: string[] = [];

  afterAll(async () => {
    for (const client of clients) await client.close().catch(() => undefined);
    writer?.close();
    if (server) {
      await server.stop();
      const outDir = process.env.LIVENESS_OUT_DIR;
      if (outDir) {
        const run = process.env.LIVENESS_RUN ?? 'run';
        fs.mkdirSync(outDir, { recursive: true });
        fs.writeFileSync(
          path.join(outDir, `query-snapshot-overflow-${run}.stdout.log`),
          server.stdout(),
        );
        fs.writeFileSync(
          path.join(outDir, `query-snapshot-overflow-${run}.notes.txt`),
          notes.join('\n') + '\n',
        );
      }
    }
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('case B folds the held writes into the snapshot; case C overflows after it and recovers', async () => {
    const { binary, commit } = requireFaultBinary();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tg-query-overflow-'));
    const holdFile = path.join(dir, 'hold');

    server = await spawnBinary({
      binaryPath: binary,
      label: 'fault',
      commit,
      env: { [HOLD_ENV]: holdFile },
    });
    const srv = server;
    const port = srv.port;
    if (port === null)
      throw new Error(`the fault server did not start:\n${srv.stdout()}\n${srv.stderr()}`);
    const lines = () => plainLines(srv.stdout());
    expect(lines().some((l) => l.includes(SEAM_BOOT_TEXT) && l.includes(HOLD_BOOT_TEXT))).toBe(
      true,
    );

    const open = async (user: string): Promise<TopGunClient> => {
      const token = createTestToken(user, ['ADMIN']);
      const client = new TopGunClient({
        serverUrl: `ws://localhost:${port}/ws`,
        storage: new MemoryStorageAdapter(),
        auth: { getToken: () => Promise.resolve(token) },
        backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
      });
      clients.push(client);
      await client.start();
      await until(
        `${user} connects`,
        () => client.getConnectionState() === SyncState.CONNECTED,
        CONNECT_WAIT_MS,
      );
      return client;
    };
    const s = await open('overflow-subscriber');
    const w = await connectWithin(port, 'overflow-writer', CONNECT_WAIT_MS);
    writer = w;

    // Spies on S: its connection state, every inbound frame in arrival order,
    // and every outbound QUERY_SUB.
    const stateChanges: StateChangeEvent[] = [];
    s.onConnectionStateChange((event) => stateChanges.push(event));
    const inbound: Inbound[] = [];
    engineOf(s).on('message', (raw: unknown) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any -- inbound frames have no single static shape
      const msg = raw as any;
      if (msg?.type === 'BATCH') return;
      const entry: Inbound = { type: msg.type, queryId: msg.payload?.queryId, events: 0 };
      if (msg.type === 'QUERY_RESP') {
        entry.code = msg.payload?.code;
        entry.results = new Map(
          (msg.payload?.results ?? []).map((r: { key: string; value: unknown }) => [
            r.key,
            r.value,
          ]),
        );
      } else if (msg.type === 'SERVER_EVENT') {
        entry.events = msg.payload?.mapName === MAP ? 1 : 0;
      } else if (msg.type === 'SERVER_BATCH_EVENT') {
        entry.events = (msg.payload?.events ?? []).filter(
          (e: { mapName?: string }) => e.mapName === MAP,
        ).length;
      }
      inbound.push(entry);
    });
    const serverEvents = () => inbound.reduce((sum, f) => sum + f.events, 0);
    // Every QUERY_SUB send carries a new wire id; each is recorded with the
    // handle it was sent for, read when the send happens.
    const outboundSubs: Array<{ id: string; handle: unknown }> = [];
    const idsOf = (handle: unknown) =>
      new Set(outboundSubs.filter((sub) => sub.handle === handle).map((sub) => sub.id));
    const wsm = engineOf(s).webSocketManager;
    const originalSend = wsm.sendMessage.bind(wsm);
    wsm.sendMessage = (
      message: { type?: string; payload?: { queryId?: string } },
      key?: string,
    ) => {
      if (message?.type === 'QUERY_SUB' && message.payload?.queryId) {
        const id = message.payload.queryId;
        outboundSubs.push({ id, handle: engineOf(s).queryManager.handleOf(id) });
      }
      return originalSend(message, key);
    };

    // One OP_BATCH per call, answered by one OP_ACK naming its last op. The
    // counter makes every stamp of W strictly newer than the one before it.
    let opSeq = 0;
    const writeAcked = async (keys: Array<[string, { v: number | string }]>) => {
      const ops = keys.map(([key, value]) => {
        opSeq += 1;
        return {
          id: `w-${opSeq}`,
          mapName: MAP,
          opType: 'PUT',
          key,
          record: { value, timestamp: { millis: Date.now(), counter: opSeq, nodeId: 'w' } },
        };
      });
      w.messages.length = 0;
      w.send({ type: 'OP_BATCH', payload: { ops } });
      const frame = await firstFrame(w, ['OP_ACK', 'OP_REJECTED', 'ERROR'], STEP_WAIT_MS);
      const lastId = ops[ops.length - 1].id;
      if (frame?.type !== 'OP_ACK' || frame.payload?.lastId !== lastId) {
        throw new Error(`W's batch ending ${lastId}: ${JSON.stringify(frame)?.slice(0, 400)}`);
      }
    };
    const seedKeys = Array.from({ length: SEED_ROWS }, (_, i) => `s-${i}`);
    await writeAcked(seedKeys.map((k, i) => [k, { v: i }]));

    /** Creates the hold file, subscribes, and waits for the hold WARN of that query id. */
    const heldSubscribe = async (label: string) => {
      fs.writeFileSync(holdFile, '');
      const createdAt = Date.now();
      const handle: QueryHandle<{ v: number | string }> = s.query(MAP, {});
      const emitted: number[] = [];
      let last: Array<{ _key: string }> = [];
      const removes: string[] = [];
      const unsubscribe = handle.subscribe((rows) => {
        last = rows as Array<{ _key: string }>;
        emitted.push(rows.length);
      });
      handle.onDelta((changes) => {
        for (const change of changes) if (change.type === 'remove') removes.push(change.key);
      });
      await until(
        `the hold WARN of ${label} (${handle.id})`,
        () =>
          lines().some((l) => {
            const id = logField(l, 'query_id');
            return l.includes(HOLD_TEXT) && id !== undefined && idsOf(handle).has(id);
          }),
        STEP_WAIT_MS,
      );
      return { handle, createdAt, emitted, removes, unsubscribe, rows: () => last };
    };
    const release = (label: string, createdAt: number) => {
      fs.unlinkSync(holdFile);
      const heldMs = Date.now() - createdAt;
      notes.push(`${label}: hold file existed ${heldMs} ms`);
      return heldMs;
    };
    const framesOf = (ids: Set<string>) =>
      inbound.filter((f) => f.queryId !== undefined && ids.has(f.queryId));
    const firstFrameOf = (ids: Set<string>) =>
      framesOf(ids).find((f) => f.type.startsWith('QUERY_'));

    // ---- Case B: the writes of the hold are folded into the snapshot. ----
    const b = await heldSubscribe('qB');
    const foldKeys = Array.from({ length: FOLD_KEYS }, (_, i) => `b-${i}`);
    await writeAcked([
      ['s-0', { v: 'updated' }],
      ...foldKeys.map((k, i): [string, { v: number }] => [k, { v: i }]),
    ]);
    const heldB = release('case B', b.createdAt);
    await until('the qB snapshot', () => firstFrameOf(idsOf(b.handle)) !== undefined, STEP_WAIT_MS);
    const firstB = firstFrameOf(idsOf(b.handle))!;
    notes.push(
      `case B frames for qB: ${framesOf(idsOf(b.handle))
        .map((f) => f.type)
        .join(',')}`,
    );
    expect(firstB.type).toBe('QUERY_RESP');
    expect(firstB.code).toBeUndefined();
    expect(firstB.results?.get('s-0')).toEqual({ v: 'updated' });
    for (const key of foldKeys) expect(firstB.results?.has(key)).toBe(true);
    expect(firstB.results?.size).toBe(SEED_ROWS + FOLD_KEYS);
    b.unsubscribe();

    // ---- Case C: a burst beyond the buffer overflows after the snapshot. ----
    const subsBefore = outboundSubs.length;
    const c = await heldSubscribe('qC');
    const overflowKeys = Array.from({ length: OVERFLOW_KEYS }, (_, i) => `c-${i}`);
    for (let at = 0; at < overflowKeys.length; at += BATCH) {
      const batch = overflowKeys.slice(at, at + BATCH);
      const eventsBefore = serverEvents();
      await writeAcked(batch.map((k, i) => [k, { v: at + i }]));
      await until(
        `S receives the SERVER_EVENTs of batch ${at / BATCH + 1}`,
        () => serverEvents() >= eventsBefore + batch.length,
        STEP_WAIT_MS,
      );
    }
    const heldC = release('case C', c.createdAt);

    const allKeys = new Set([...seedKeys, ...foldKeys, ...overflowKeys]);
    await until(
      'qC ends with every key of the map',
      () => c.rows().length === allKeys.size && c.rows().every((r) => allKeys.has(r._key)),
      RESUBSCRIBE_WAIT_MS,
    );

    const responses = framesOf(idsOf(c.handle)).filter((f) => f.type === 'QUERY_RESP');
    notes.push(
      `case C frames for qC: ${framesOf(idsOf(c.handle))
        .map((f) => `${f.type}${f.code ? `(${f.code})` : ''}[${f.results?.size ?? ''}]`)
        .join(',')}`,
    );
    notes.push(`case C SERVER_EVENTs received: ${serverEvents()}`);
    notes.push(`case C emitted row counts: ${c.emitted.join(',')}`);
    notes.push(`state changes: ${JSON.stringify(stateChanges)}`);

    // The snapshot, then the overflow, then the resubscribe's snapshot.
    expect(firstFrameOf(idsOf(c.handle))?.type).toBe('QUERY_RESP');
    expect(responses.length).toBeGreaterThanOrEqual(3);
    expect(responses[0].code).toBeUndefined();
    expect(responses[1].code).toBe(OVERFLOW_CODE);
    expect(responses[2].code).toBeUndefined();
    expect(framesOf(idsOf(c.handle)).some((f) => f.type === 'QUERY_UPDATE')).toBe(false);
    const subsOfC = outboundSubs.slice(subsBefore);
    expect(subsOfC.map((sub) => sub.handle === c.handle)).toEqual([true, true]);
    expect(new Set(subsOfC.map((sub) => sub.id)).size).toBe(2);
    expect(subsOfC.some((sub) => sub.id === c.handle.id)).toBe(false);
    expect(new Set(c.rows().map((r) => r._key))).toEqual(allKeys);
    // Once qC has emitted rows, it never shows fewer.
    const fromFirstRows = c.emitted.slice(c.emitted.findIndex((n) => n > 0));
    for (let i = 1; i < fromFirstRows.length; i++) {
      expect(fromFirstRows[i]).toBeGreaterThanOrEqual(fromFirstRows[i - 1]);
    }
    expect(c.removes).toEqual([]);
    c.unsubscribe();

    // Isolation: the holds harmed nobody.
    expect(stateChanges).toEqual([]);
    expect(lines().some((l) => l.includes(SLOW_CONSUMER_TEXT))).toBe(false);
    expect(lines().some((l) => l.includes(LIVE_LOSS_TEXT))).toBe(false);
    expect(heldB).toBeLessThanOrEqual(MAX_HOLD_MS);
    expect(heldC).toBeLessThanOrEqual(MAX_HOLD_MS);
    expect(await metricLine(port, OVERFLOW_SERIES)).toBe(`${OVERFLOW_SERIES} 1`);
  }, 120_000);
});
