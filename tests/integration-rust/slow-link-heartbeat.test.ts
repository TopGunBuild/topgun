/**
 * Integration: a TopGunClient on a slow but live link does not reconnect while
 * a large live snapshot arrives, and a dead link is still detected within the
 * heartbeat's bound (TG-WS-002, TG-WS-003).
 *
 * The client is a TopGunClient in a process of its own
 * (helpers/client-subscriber-process.js) holding a live query of one map. It
 * reaches the real server through helpers/throttle-proxy.ts, which caps each
 * connection's server-to-client byte rate. A disconnect is any transition into
 * DISCONNECTED, BACKOFF or ERROR after the client first reached CONNECTED.
 *
 * - A: a ~4.4 MB snapshot (4 000 rows of ~1 KiB) at 1 Mbit/s, 3 runs: it
 *   completes within 120 s with no disconnect, and it took at least 0.8 of the
 *   time the capped bytes need, so the cap was real.
 * - B: a ~512 KiB snapshot (450 rows) at 0.4 Mbit/s, 3 runs, the same checks.
 * - C: a dead idle link. With the snapshot complete on an unthrottled proxy,
 *   the path is blackholed at B0; the client disconnects within
 *   [B0 + 10 s, B0 + 21 s] (its last inbound message is a PONG of the last 5 s)
 *   and, within 30 s, is connected again with a fresh server result.
 * - D: a dead link in the middle of a transfer. At 1 Mbit/s the path is
 *   blackholed 10 s after CONNECTED; the client disconnects within
 *   [B0 + 14 s, B0 + 21 s] (a CHUNK arrived every ~0.5 s up to B0) and, within
 *   90 s of connecting again, holds every row and a server result newer than
 *   the disconnect, with no further disconnect.
 *
 * Environment:
 *
 * - SLOW_LINK_RATES  — comma list of Mbit/s for case A (default `1`); the
 *   evidence arms are `0.3,3.5`;
 * - SLOW_LINK_RUNS   — runs per rate of A and of B (default 3);
 * - SLOW_LINK_OUT    — a JSONL file each run appends its record to;
 * - SLOW_LINK_COMMIT — the server's commit, written to the record.
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

import { SyncState } from '@topgunbuild/client';

import { spawnRustServer, createRustTestClient, TestClient } from './helpers';
import { startThrottleProxy, ThrottleProxy } from './helpers/throttle-proxy';

const MAP_4000 = 'slow-link-4000';
const MAP_450 = 'slow-link-450';
const PAD = 'x'.repeat(1000);
const RUNS = Number(process.env.SLOW_LINK_RUNS ?? 3);
const A_RATES = (process.env.SLOW_LINK_RATES ?? '1')
  .split(',')
  .map((r) => Number(r.trim()))
  .filter((r) => r > 0);
const DISCONNECTED = new Set<string>([SyncState.DISCONNECTED, SyncState.BACKOFF, SyncState.ERROR]);

interface Change {
  from: string;
  to: string;
  at: number;
}

interface Status {
  keys: number;
  state: string;
  changes: Change[];
  serverResults: number;
  lastServerResultAt: number | null;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Polls `condition` every `everyMs` until it holds or `withinMs` passes; returns whether it held. */
async function poll(condition: () => boolean, withinMs: number, everyMs = 50): Promise<boolean> {
  const until = Date.now() + withinMs;
  while (Date.now() < until) {
    if (condition()) return true;
    await sleep(everyMs);
  }
  return condition();
}

function record(entry: Record<string, unknown>): void {
  const out = process.env.SLOW_LINK_OUT;
  const line = JSON.stringify({ ...entry, commit: process.env.SLOW_LINK_COMMIT ?? null });
  console.log(`slow-link ${line}`);
  if (out) fs.appendFileSync(out, `${line}\n`);
}

function disconnectsOf(status: Status | null): Change[] {
  return (status?.changes ?? []).filter((c) => DISCONNECTED.has(c.to));
}

/** Writes `rows` rows with OP_BATCHes of 100 ops, waiting for each batch's OP_ACK. */
async function writeRows(writer: TestClient, mapName: string, rows: number): Promise<void> {
  for (let at = 0; at < rows; at += 100) {
    const ops = [];
    for (let i = at; i < Math.min(at + 100, rows); i++) {
      ops.push({
        id: `${mapName}-${i}`,
        mapName,
        opType: 'PUT',
        key: `k${i}`,
        record: {
          value: { i, pad: PAD },
          timestamp: { millis: Date.now(), counter: i, nodeId: 'slow-link-writer' },
        },
      });
    }
    writer.messages.length = 0;
    writer.send({ type: 'OP_BATCH', payload: { ops } });
    const ack = await writer.waitForMessage('OP_ACK', 10_000);
    expect(ack.type).toBe('OP_ACK');
  }
}

/** The TopGunClient subscriber process, dialing the proxy. */
class Subscriber {
  readonly proc: child_process.ChildProcess;
  status: Status | null = null;
  private exited = false;

  constructor(port: number, mapName: string) {
    this.proc = child_process.fork(
      path.join(__dirname, 'helpers', 'client-subscriber-process.js'),
      [String(port), mapName],
      { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] },
    );
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC messages of the subscriber process
    this.proc.on('message', (message: any) => {
      if (message?.type === 'status') this.status = message as Status;
    });
    this.proc.on('exit', () => {
      this.exited = true;
    });
  }

  async stop(): Promise<void> {
    if (this.exited) return;
    this.proc.send({ type: 'exit' });
    if (!(await poll(() => this.exited, 5_000))) this.proc.kill('SIGKILL');
  }
}

describe('Integration: client heartbeat on a slow link (Rust Server)', () => {
  let port: number;
  let cleanup: () => Promise<void>;
  let writer: TestClient;
  let proxy: ThrottleProxy | null = null;
  let subscriber: Subscriber | null = null;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
    writer = await createRustTestClient(port, {
      nodeId: 'slow-link-writer',
      userId: 'slow-link-writer',
      roles: ['ADMIN'],
    });
    await writer.waitForMessage('AUTH_ACK', 10_000);
    await writeRows(writer, MAP_4000, 4000);
    await writeRows(writer, MAP_450, 450);
  }, 120_000);

  afterEach(async () => {
    await subscriber?.stop();
    subscriber = null;
    await proxy?.close();
    proxy = null;
  });

  afterAll(async () => {
    writer?.close();
    await cleanup?.();
  });

  /** Opens a proxy at `mbps` and a subscriber of `mapName` behind it; returns when it is CONNECTED. */
  async function open(
    mbps: number,
    mapName: string,
  ): Promise<{ s: Subscriber; connectedAt: number }> {
    proxy = await startThrottleProxy({
      targetPort: port,
      bitsPerSecond: mbps === Infinity ? Infinity : mbps * 1e6,
    });
    subscriber = new Subscriber(proxy.port, mapName);
    const s = subscriber;
    const connected = await poll(() => s.status?.state === SyncState.CONNECTED, 30_000);
    expect(connected).toBe(true);
    return { s, connectedAt: Date.now() };
  }

  /** One run of A or B: the snapshot completes, with no disconnect, at the capped rate. */
  async function snapshotRun(
    kase: string,
    run: number,
    mbps: number,
    mapName: string,
    rows: number,
    minBytes: number,
    withinMs: number,
  ): Promise<void> {
    const { s, connectedAt } = await open(mbps, mapName);
    const complete = await poll(() => (s.status?.keys ?? 0) >= rows, withinMs, 100);
    const elapsedMs = Date.now() - connectedAt;
    const bytes = proxy!.bytesDownstream();
    const minMs = ((0.8 * bytes * 8) / (mbps * 1e6)) * 1000;
    const disconnects = disconnectsOf(s.status).length;
    record({
      case: kase,
      run,
      mbps,
      keys: s.status?.keys ?? 0,
      complete,
      disconnects,
      bytes,
      elapsedMs,
      minMs,
    });
    expect(complete).toBe(true);
    expect(disconnects).toBe(0);
    expect(bytes).toBeGreaterThanOrEqual(minBytes);
    expect(elapsedMs).toBeGreaterThanOrEqual(minMs);
    await subscriber?.stop();
    subscriber = null;
    await proxy?.close();
    proxy = null;
  }

  for (const mbps of A_RATES) {
    // 120 s at 1 Mbit/s, the transfer time twice over on slower links.
    const withinMs = Math.max(120_000, (2 * 4_400_000 * 8 * 1000) / (mbps * 1e6));
    test(
      `A: a ~4.4 MB live snapshot at ${mbps} Mbit/s completes with no disconnect, ${RUNS} of ${RUNS}`,
      async () => {
        for (let run = 0; run < RUNS; run++) {
          await snapshotRun('A', run, mbps, MAP_4000, 4000, 4_000_000, withinMs);
        }
      },
      RUNS * (withinMs + 45_000),
    );
  }

  test(
    `B: a ~512 KiB live snapshot at 0.4 Mbit/s completes with no disconnect, ${RUNS} of ${RUNS}`,
    async () => {
      for (let run = 0; run < RUNS; run++) {
        await snapshotRun('B', run, 0.4, MAP_450, 450, 512_000, 120_000);
      }
    },
    RUNS * 165_000,
  );

  test('C: a dead idle link is detected within the heartbeat bound, then a fresh result arrives', async () => {
    const { s } = await open(Infinity, MAP_4000);
    expect(
      await poll(
        () => (s.status?.keys ?? 0) >= 4000 && (s.status?.serverResults ?? 0) >= 1,
        60_000,
      ),
    ).toBe(true);
    const resultsAtB0 = s.status!.serverResults;
    const b0 = Date.now();
    proxy!.blackhole();
    const lost = await poll(() => disconnectsOf(s.status).some((c) => c.at > b0), 30_000);
    const disconnectAt = disconnectsOf(s.status).find((c) => c.at > b0)?.at ?? null;
    const fresh = () =>
      disconnectAt !== null &&
      (s.status?.changes ?? []).some((c) => c.to === SyncState.CONNECTED && c.at > disconnectAt) &&
      (s.status?.serverResults ?? 0) > resultsAtB0 &&
      (s.status?.lastServerResultAt ?? 0) > disconnectAt;
    const recovered = lost && (await poll(fresh, Math.max(0, disconnectAt! + 30_000 - Date.now())));
    const reconnectedAt =
      (s.status?.changes ?? []).find(
        (c) => c.to === SyncState.CONNECTED && c.at > (disconnectAt ?? 0),
      )?.at ?? null;
    record({
      case: 'C',
      detectMs: disconnectAt === null ? null : disconnectAt - b0,
      reconnectMs:
        reconnectedAt === null || disconnectAt === null ? null : reconnectedAt - disconnectAt,
      recovered,
      serverResults: s.status?.serverResults,
      resultsAtB0,
    });
    expect(disconnectAt).not.toBeNull();
    expect(disconnectAt! - b0).toBeGreaterThanOrEqual(10_000);
    expect(disconnectAt! - b0).toBeLessThanOrEqual(21_000);
    expect(recovered).toBe(true);
  }, 120_000);

  test('D: a link that dies mid-transfer is detected within the bound, then the snapshot completes', async () => {
    const { s, connectedAt } = await open(1, MAP_4000);
    await sleep(Math.max(0, connectedAt + 10_000 - Date.now()));
    const keysAtB0 = s.status?.keys ?? 0;
    const b0 = Date.now();
    proxy!.blackhole();
    expect(keysAtB0).toBeLessThan(4000);
    const lost = await poll(() => disconnectsOf(s.status).some((c) => c.at > b0), 30_000);
    const disconnectAt = disconnectsOf(s.status).find((c) => c.at > b0)?.at ?? null;
    const back = await poll(
      () =>
        disconnectAt !== null &&
        (s.status?.changes ?? []).some((c) => c.to === SyncState.CONNECTED && c.at > disconnectAt),
      30_000,
    );
    const reconnectedAt =
      (s.status?.changes ?? []).find(
        (c) => c.to === SyncState.CONNECTED && c.at > (disconnectAt ?? 0),
      )?.at ?? null;
    const complete =
      lost &&
      back &&
      (await poll(
        () => (s.status?.keys ?? 0) >= 4000 && (s.status?.lastServerResultAt ?? 0) > disconnectAt!,
        Math.max(0, reconnectedAt! + 90_000 - Date.now()),
        100,
      ));
    const disconnects = disconnectsOf(s.status).length;
    record({
      case: 'D',
      keysAtB0,
      detectMs: disconnectAt === null ? null : disconnectAt - b0,
      completeMs: complete && reconnectedAt !== null ? Date.now() - reconnectedAt : null,
      complete,
      disconnects,
    });
    expect(disconnectAt).not.toBeNull();
    expect(disconnectAt! - b0).toBeGreaterThanOrEqual(14_000);
    expect(disconnectAt! - b0).toBeLessThanOrEqual(21_000);
    expect(complete).toBe(true);
    expect(disconnects).toBe(1);
  }, 180_000);
});
