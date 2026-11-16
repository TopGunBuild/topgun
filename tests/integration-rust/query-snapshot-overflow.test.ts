/**
 * Integration: a live query whose snapshot overflows keeps its rows and gets
 * back every write the overflow dropped.
 *
 * While a live subscribe builds its snapshot the server captures the map's
 * writes in a bounded delta buffer (4096 distinct keys). Past that bound the
 * captured writes are dropped and the server sends a QUERY_RESP with code
 * QUERY_SNAPSHOT_OVERFLOW ahead of the snapshot. A client that applied that
 * frame as a result set wiped its rows, and the dropped writes stayed missing
 * until their keys changed again. This test forces a real overflow and checks
 * that the handle never loses its rows, resubscribes once, and ends with the
 * server's rows.
 *
 * The overflow window is microseconds in a release build, so the server is a
 * `fault-injection` build: while the file named by
 * TOPGUN_FAULT_QUERY_DRAIN_HOLD_FILE exists, a live subscribe waits between its
 * snapshot and its drain. No release build has that seam, so the binary is
 * given by path (FAULT_SERVER_BINARY) with the commit it was built from
 * (FAULT_SERVER_COMMIT). With either missing the test fails; it never skips.
 *
 * Isolation from reconnects: a reconnect would resubscribe every query and
 * count as a second QUERY_SUB for the wrong reason. The hold stalls the global
 * worker, which also answers PING, and the writer's 5000 writes reach the
 * subscriber as SERVER_EVENTs that could fill its outbound channel. Both
 * clients keep the default heartbeat, and the writer is paced instead (50
 * batches of 100, each acknowledged before the next), which bounds the hold
 * and the burst at once. An acknowledgement bounds what the server enqueued to
 * the subscriber, not what it received; the slow-consumer check below is what
 * catches a channel that does not drain.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { ChangeEvent, QueryHandle } from '@topgunbuild/client';

import { createTestToken, MemoryStorageAdapter } from './helpers';
import { requireFaultBinary } from './helpers/fault-binary';
import { spawnBinary, plainLines, sleep, SpawnedBinary } from './helpers/two-binary';

const MAP = 'overflow-map';
const BASELINE = 50;
const BATCHES = 50;
const PER_BATCH = 100;
const HOLD_WARN = 'fault injection: live query drain held';
const SLOW_CONSUMER = 'slow consumer exceeded broadcast drop threshold';
// Half the default heartbeat timeout (15 s): a longer hold could make a client
// reconnect, which would resubscribe every query for a reason other than the overflow.
const MAX_HOLD_MS = 7_500;

jest.setTimeout(120_000);

async function waitUntil(what: string, check: () => boolean, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!check()) {
    if (Date.now() > deadline)
      throw new Error(`timed out after ${timeoutMs} ms waiting for ${what}`);
    await sleep(20);
  }
}

async function connectedClient(port: number, user: string): Promise<TopGunClient> {
  const token = createTestToken(user, ['ADMIN']);
  const client = new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
  });
  await client.start();
  await waitUntil(
    `${user} to connect`,
    () => client.getConnectionState() === SyncState.CONNECTED,
    20_000,
  );
  return client;
}

describe('Integration: live query snapshot overflow (fault-injection Rust server)', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'query-overflow-'));
  const holdFile = path.join(tmp, 'hold');
  let server: SpawnedBinary | undefined;
  const clients: TopGunClient[] = [];

  afterAll(async () => {
    fs.rmSync(holdFile, { force: true });
    for (const client of clients) await client.close().catch(() => {});
    await server?.stop();
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  test('the handle keeps its rows, resubscribes once and ends with the server rows', async () => {
    // Step 1: the fault binary, with the hold file absent.
    const { binary, commit } = requireFaultBinary();
    server = await spawnBinary({
      binaryPath: binary,
      commit,
      label: 'query-snapshot-overflow',
      env: { TOPGUN_FAULT_QUERY_DRAIN_HOLD_FILE: holdFile },
    });
    if (server.port === null) {
      throw new Error(`the server did not print PORT=\n--- stderr ---\n${server.stderr()}`);
    }
    const port = server.port;
    const stdout = () => plainLines(server!.stdout());

    // Step 2: both clients connected, map opened and synced before the hold exists.
    const s = await connectedClient(port, 'overflow-subscriber');
    const w = await connectedClient(port, 'overflow-writer');
    clients.push(s, w);
    const sMap = s.getMap<string, { n: number }>(MAP);
    const wMap = w.getMap<string, { b: number; i: number }>(MAP);
    await sleep(500);
    const sStates: string[] = [];
    const wStates: string[] = [];
    s.onConnectionStateChange((event) => sStates.push(`${event.from}->${event.to}`));
    w.onConnectionStateChange((event) => wStates.push(`${event.from}->${event.to}`));

    // Step 3: baseline rows, acknowledged by the server.
    const baseline = Array.from({ length: BASELINE }, (_, i) => `base-${i}`);
    baseline.forEach((key, i) => sMap.set(key, { n: i }));
    await waitUntil('the baseline to be acknowledged', () => s.getPendingOpsCount() === 0, 10_000);

    // Step 4: the hold, then the live query. QueryManager sends through this function.
    fs.writeFileSync(holdFile, '');
    const wsm = (s as unknown as { syncEngine: { webSocketManager: object } }).syncEngine
      .webSocketManager as { sendMessage: (message: unknown, key?: string) => boolean };
    const sendSpy = jest.spyOn(wsm, 'sendMessage');
    const handle: QueryHandle<{ n?: number }> = s.query(MAP, {});
    const emissions: Array<{ keys: Set<string>; settled: boolean }> = [];
    const deltas: ChangeEvent<{ n?: number }>[] = [];
    handle.subscribe((rows, meta) => {
      emissions.push({
        keys: new Set(rows.map((row) => row._key)),
        settled: meta?.settled === true,
      });
    });
    handle.onDelta((changes) => deltas.push(...changes));
    const querySubs = () =>
      sendSpy.mock.calls.filter(([message]) => {
        const m = message as { type?: string; payload?: { queryId?: string } };
        return m.type === 'QUERY_SUB' && m.payload?.queryId === handle.id;
      }).length;

    await waitUntil(
      'the drain hold WARN line',
      () => stdout().some((l) => l.includes(HOLD_WARN)),
      10_000,
    );
    const heldAt = Date.now();

    // Step 5: 5000 new keys, paced, each batch acknowledged before the next.
    for (let b = 0; b < BATCHES; b++) {
      for (let i = 0; i < PER_BATCH; i++) wMap.set(`w-${b}-${i}`, { b, i });
      await waitUntil(`batch ${b} to be acknowledged`, () => w.getPendingOpsCount() === 0, 10_000);
    }

    // Step 6: release the hold.
    fs.rmSync(holdFile);
    const releasedAt = Date.now();
    const holdMs = releasedAt - heldAt;

    // (h) checked first, so a slow run names the pacing instead of a symptom.
    if (holdMs > MAX_HOLD_MS) {
      throw new Error(
        `the hold window was ${holdMs} ms, over ${MAX_HOLD_MS} ms (half the heartbeat timeout): ` +
          `the writer's pacing (${BATCHES} acknowledged batches of ${PER_BATCH}) took too long on ` +
          `this host, so a reconnect could have resubscribed the query; this is not a flake`,
      );
    }

    // Step 7: the handle resubscribes and converges.
    const total = BASELINE + BATCHES * PER_BATCH;
    await waitUntil(
      `${total} rows on the handle`,
      () => emissions.length > 0 && emissions[emissions.length - 1].keys.size === total,
      15_000,
    );

    // (a) no wipe: from the first 50-row emission on, every emission holds every baseline key.
    const first = emissions.findIndex((e) => e.keys.size === BASELINE);
    expect(first).toBeGreaterThanOrEqual(0);
    for (const emission of emissions.slice(first)) {
      for (const key of baseline) expect(emission.keys.has(key)).toBe(true);
    }
    const baselineRemoves = deltas.filter((d) => d.type === 'remove' && baseline.includes(d.key));
    expect(baselineRemoves).toEqual([]);

    // (b) the overflow really dropped the writes: a settled 50-row emission came first.
    const settledBaseline = emissions.findIndex((e) => e.settled && e.keys.size === BASELINE);
    expect(settledBaseline).toBeGreaterThanOrEqual(0);
    expect(settledBaseline).toBeLessThan(emissions.length - 1);

    // (c) the final rows are the server's, read by a fresh client.
    const fresh = await connectedClient(port, 'overflow-reader');
    clients.push(fresh);
    const truth = await fresh.queryOnce<{ n?: number }>(MAP, {});
    const truthKeys = new Set(truth.map((row) => row._key));
    expect(truthKeys.size).toBe(total);
    expect([...emissions[emissions.length - 1].keys].sort()).toEqual([...truthKeys].sort());

    // (d) the overflow reached the client and exactly one resubscribe was sent.
    expect(querySubs()).toBe(2);
    // (e) recovered: no error left on the handle.
    expect(handle.getError()).toBeNull();
    // (f) neither client changed connection state after step 2.
    expect({ s: sStates, w: wStates }).toEqual({ s: [], w: [] });
    // (g) the subscriber's channel drained: no slow-consumer disconnect.
    expect(stdout().filter((line) => line.includes(SLOW_CONSUMER))).toEqual([]);
  });
});
