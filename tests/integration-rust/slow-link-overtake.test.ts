/**
 * Integration: on a slow but live link, small messages pass a long live
 * snapshot, so live writes during the transfer cause no reconnect and other
 * answers arrive before it completes (TG-WS-004).
 *
 * The client is a TopGunClient in a process of its own
 * (helpers/client-subscriber-process.js) holding a live query of
 * `overtake-4000` (4 000 rows of ~1 KiB, a ~4.1 MB snapshot). It reaches the
 * real server through helpers/throttle-proxy.ts with a 16 KiB high-water mark,
 * so the proxy models a link rather than adding a deep buffer. A disconnect is
 * any transition into DISCONNECTED, BACKOFF or ERROR after the client first
 * reached CONNECTED. "Fresh" rows are a `queryOnce` of a second client
 * connected to the server directly, taken once the writes have stopped.
 *
 * - E: live writes at R writes/s (alternately a new key and an overwrite of a
 *   seeded key) during the transfer: no disconnect, no owed-loss disconnect on
 *   the server, the snapshot took at least 0.8 of the time the capped bytes
 *   need, and the child's rows end equal to the fresh rows.
 * - F: a `queryOnce` of `overtake-small` 8 s into the transfer settles, with
 *   its 10 rows, before 0.9 of the run's downstream bytes have passed.
 * - G: a write to `overtake-small`, live-queried by the child as a second
 *   query, 8 s into the transfer reaches the child while the 4 000 seeded keys
 *   have not all arrived.
 * - H: a child that stops reading mid-transfer while writes go on at 20/s is
 *   disconnected by the owed-loss rule (TG-LIVE-003) within 30 s, and once it
 *   reads again its rows converge to the fresh rows.
 *
 * Latency bound. In the worst case the bytes ahead of a small message queued at
 * the server are the rest of one CHUNK (C + 64), the server socket's unsent
 * bytes (L + G: 16 384 + 65 536 on Linux, where TCP_NOTSENT_LOWAT is set), the
 * proxy's upstream receive buffer R_p (the middle value of tcp_rmem) and the
 * proxy's queue (16 384 + one 65 536-byte read):
 * D(rate) = (C + 64 + L + G + R_p + 16 384 + 65 536) × 8 / rate + 0.5 s, the
 * 0.5 s for the child's event loop, its 100 ms status interval and IPC. F's and
 * G's latencies are asserted against D only on Linux. So are F's and G's
 * end-to-end ordering and H: they depend on the buffers below the writer, and
 * on macOS the server's send buffer and the proxy's and the child's receive
 * buffers are auto-tuned up to 4 MiB, enough to hold most or all of the
 * answer (the writer's order is shown on every platform by its unit tests).
 * Elsewhere the values are recorded, beside the Linux figure. On Linux each F
 * and G record carries `ss -tmi` of the server's and the proxy's sockets at
 * that moment.
 *
 * Environment:
 *
 * - SLOW_LINK_RATES    — comma list of Mbit/s (default `1`). At 1 Mbit/s the
 *   file runs E (R = 7 and 20), F (default 5 s timeout), G and H; at any other
 *   rate, the evidence arms: E (R = 2, 7 and 20) and F with a 30 s timeout;
 * - SLOW_LINK_E_WRITES — comma list overriding E's write rates;
 * - SLOW_LINK_RUNS     — runs of E (per rate and R), F and G (default 3);
 * - SLOW_LINK_OUT      — a JSONL file each run appends its record to;
 * - SLOW_LINK_COMMIT   — the server's commit, written to the record.
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { SyncState, TopGunClient } from '@topgunbuild/client';

import { spawnRustServer, createRustTestClient, TestClient } from './helpers';
import { MemoryStorageAdapter } from './helpers/memory-storage';
import { createTestToken } from './helpers/test-client';
import { startThrottleProxy, ThrottleProxy } from './helpers/throttle-proxy';

const MAP = 'overtake-4000';
const SMALL = 'overtake-small';
const SEED_ROWS = 4000;
const SMALL_ROWS = 10;
const PAD = 'x'.repeat(1000);
const WRITE_PAD = 'w'.repeat(80);
const RUNS = Number(process.env.SLOW_LINK_RUNS ?? 3);
const RATES = (process.env.SLOW_LINK_RATES ?? '1')
  .split(',')
  .map((r) => Number(r.trim()))
  .filter((r) => r > 0);
const E_WRITES_OVERRIDE = process.env.SLOW_LINK_E_WRITES?.split(',')
  .map((r) => Number(r.trim()))
  .filter((r) => r > 0);
const DISCONNECTED = new Set<string>([SyncState.DISCONNECTED, SyncState.BACKOFF, SyncState.ERROR]);
const LOSS_SERIES = 'topgun_live_event_loss_disconnects_total{cause="channel_full"}';

/** The chunk length, the server's unsent-byte bound and one send's segment. */
const C = 65_536;
const L = 16_384;
const G = 65_536;
const PROXY_HIGH_WATER = 16_384;
const PROXY_READ = 65_536;

interface Change {
  from: string;
  to: string;
  at: number;
}

interface Status {
  keys: number;
  state: string;
  changes: Change[];
  keys2: number;
  prefixKeys: number;
}

interface QueryOnceResult {
  ok: boolean;
  ms: number;
  rows?: unknown[];
  error?: string;
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
  console.log(`slow-link-overtake ${line}`);
  if (out) fs.appendFileSync(out, `${line}\n`);
}

function disconnectsOf(status: Status | null): Change[] {
  return (status?.changes ?? []).filter((c) => DISCONNECTED.has(c.to));
}

/** The proxy's upstream receive buffer: Linux's default `tcp_rmem`, else Linux's usual default. */
function proxyReceiveBuffer(): number {
  if (process.platform === 'linux') {
    const fields = fs.readFileSync('/proc/sys/net/ipv4/tcp_rmem', 'utf8').trim().split(/\s+/);
    return Number(fields[1]);
  }
  return 131_072;
}

/** The latency bound D(rate) in ms, from the parts listed in the header. */
function bound(mbps: number): number {
  const bytes = C + 64 + L + G + proxyReceiveBuffer() + PROXY_HIGH_WATER + PROXY_READ;
  return ((bytes * 8) / (mbps * 1e6)) * 1000 + 500;
}

const LINUX = process.platform === 'linux';

/** `ss -tmi` of the sockets of `ports` on Linux, else null; null too where `ss` is missing. */
function socketStats(ports: number[]): string | null {
  if (!LINUX) return null;
  const filter = ports.map((p) => `sport = :${p} or dport = :${p}`).join(' or ');
  try {
    return child_process.execFileSync('ss', ['-tmi', `( ${filter} )`], { encoding: 'utf8' });
  } catch {
    return null;
  }
}

async function lossCount(port: number): Promise<number> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${LOSS_SERIES} `));
  if (line === undefined) throw new Error(`/metrics has no series ${LOSS_SERIES}`);
  return Number(line.slice(LOSS_SERIES.length + 1));
}

let opSeq = 0;

/** Writes `ops` as one OP_BATCH and waits for its OP_ACK. */
async function writeAcked(
  writer: TestClient,
  mapName: string,
  rows: Array<[string, Record<string, unknown>]>,
): Promise<void> {
  const ops = rows.map(([key, value]) => {
    opSeq += 1;
    return {
      id: `overtake-${opSeq}`,
      mapName,
      opType: 'PUT',
      key,
      record: {
        value,
        timestamp: { millis: Date.now(), counter: opSeq, nodeId: 'overtake-writer' },
      },
    };
  });
  writer.messages.length = 0;
  writer.send({ type: 'OP_BATCH', payload: { ops } });
  const ack = await writer.waitForMessage('OP_ACK', 10_000);
  expect(ack.type).toBe('OP_ACK');
}

/**
 * Writes at `perSecond` until `stop()` holds, alternating a new key
 * `{prefix}-{i}` and an overwrite of a seeded key chosen by a seeded generator.
 */
async function writeAt(
  writer: TestClient,
  perSecond: number,
  prefix: string,
  stop: () => boolean,
): Promise<number> {
  let state = 12_345;
  const next = () => {
    state = (state * 1_103_515_245 + 12_345) % 2 ** 31;
    return state;
  };
  const started = Date.now();
  let written = 0;
  while (!stop()) {
    const key = written % 2 === 0 ? `${prefix}-${written}` : `k-${next() % SEED_ROWS}`;
    await writeAcked(writer, MAP, [[key, { w: written, prefix, pad: WRITE_PAD }]]);
    written += 1;
    await sleep(Math.max(0, started + (written * 1000) / perSecond - Date.now()));
  }
  return written;
}

/** Rows as `key → JSON of the value`, sorted by key, so two views compare as strings. */
function canonical(rows: Array<[string, unknown]>): string {
  const sortedValue = (value: unknown): unknown => {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return value;
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .filter(([k]) => k !== '_key')
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([k, v]) => [k, sortedValue(v)]),
    );
  };
  return JSON.stringify(
    rows
      .map(([key, value]) => [key, sortedValue(value)] as [string, unknown])
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}

/** The TopGunClient subscriber process, dialing the proxy. */
class Subscriber {
  readonly proc: child_process.ChildProcess;
  status: Status | null = null;
  private exited = false;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC replies of the subscriber process
  private waiters: Array<{ type: string; resolve: (message: any) => void }> = [];

  constructor(port: number, mapName: string) {
    this.proc = child_process.fork(
      path.join(__dirname, 'helpers', 'client-subscriber-process.js'),
      [String(port), mapName],
      { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] },
    );
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC messages of the subscriber process
    this.proc.on('message', (message: any) => {
      if (message?.type === 'status') this.status = message as Status;
      const at = this.waiters.findIndex((w) => w.type === message?.type);
      if (at >= 0) this.waiters.splice(at, 1)[0].resolve(message);
    });
    this.proc.on('exit', () => {
      this.exited = true;
    });
  }

  send(message: Record<string, unknown>): void {
    this.proc.send(message);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC replies of the subscriber process
  next(type: string): Promise<any> {
    return new Promise((resolve) => this.waiters.push({ type, resolve }));
  }

  async rows(): Promise<Array<[string, unknown]>> {
    const reply = this.next('rows');
    this.send({ type: 'dump' });
    return (await reply).rows;
  }

  async stop(): Promise<void> {
    if (this.exited) return;
    this.proc.send({ type: 'exit' });
    if (!(await poll(() => this.exited, 5_000))) this.proc.kill('SIGKILL');
  }
}

describe('Integration: small messages pass a long answer on a slow link (Rust Server)', () => {
  let port: number;
  let cleanup: () => Promise<void>;
  let writer: TestClient;
  let fresh: TopGunClient;
  let proxy: ThrottleProxy | null = null;
  let subscriber: Subscriber | null = null;
  // The proxy's downstream bytes every 500 ms since the subscriber connected,
  // recorded with F and G so a latency miss can be read against the transfer.
  let trace: Array<[number, number]> = [];
  let tracer: ReturnType<typeof setInterval> | null = null;

  beforeAll(async () => {
    const server = await spawnRustServer();
    port = server.port;
    cleanup = server.cleanup;
    writer = await createRustTestClient(port, {
      nodeId: 'overtake-writer',
      userId: 'overtake-writer',
      roles: ['ADMIN'],
    });
    await writer.waitForMessage('AUTH_ACK', 10_000);
    for (let at = 0; at < SEED_ROWS; at += 100) {
      const rows: Array<[string, Record<string, unknown>]> = [];
      for (let i = at; i < at + 100; i++) rows.push([`k-${i}`, { i, pad: PAD }]);
      await writeAcked(writer, MAP, rows);
    }
    await writeAcked(
      writer,
      SMALL,
      Array.from({ length: SMALL_ROWS }, (_, i): [string, Record<string, unknown>] => [
        `s-${i}`,
        { i },
      ]),
    );
    const token = createTestToken('overtake-fresh', ['ADMIN']);
    fresh = new TopGunClient({
      serverUrl: `ws://127.0.0.1:${port}/ws`,
      storage: new MemoryStorageAdapter(),
      auth: { getToken: async () => token },
    });
    void fresh.start();
    expect(await poll(() => fresh.getConnectionState() === SyncState.CONNECTED, 30_000)).toBe(true);
    record({
      case: 'setup',
      platform: process.platform,
      proxyReceiveBuffer: proxyReceiveBuffer(),
      bound1MbitMs: bound(1),
      bound03MbitMs: bound(0.3),
    });
  }, 180_000);

  afterEach(async () => {
    if (tracer) clearInterval(tracer);
    tracer = null;
    await subscriber?.stop();
    subscriber = null;
    await proxy?.close();
    proxy = null;
  });

  afterAll(async () => {
    await fresh?.close().catch(() => {});
    writer?.close();
    await cleanup?.();
  });

  /** Opens a proxy at `mbps` and a subscriber of MAP behind it; returns when it is CONNECTED. */
  async function open(mbps: number): Promise<{ s: Subscriber; connectedAt: number }> {
    proxy = await startThrottleProxy({
      targetPort: port,
      bitsPerSecond: mbps * 1e6,
      highWaterBytes: PROXY_HIGH_WATER,
    });
    subscriber = new Subscriber(proxy.port, MAP);
    const s = subscriber;
    expect(await poll(() => s.status?.state === SyncState.CONNECTED, 30_000)).toBe(true);
    const connectedAt = Date.now();
    trace = [];
    if (tracer) clearInterval(tracer);
    const p = proxy;
    tracer = setInterval(() => trace.push([Date.now() - connectedAt, p.bytesDownstream()]), 500);
    return { s, connectedAt };
  }

  async function close(): Promise<void> {
    if (tracer) clearInterval(tracer);
    tracer = null;
    await subscriber?.stop();
    subscriber = null;
    await proxy?.close();
    proxy = null;
  }

  async function freshRows(): Promise<Array<[string, unknown]>> {
    const rows = await fresh.queryOnce<Record<string, unknown>>(MAP, {}, { timeoutMs: 30_000 });
    return rows.map((row) => [String(row._key), row]);
  }

  /** Polls until the child's rows equal the fresh rows, or `withinMs` passes. */
  async function converge(s: Subscriber, withinMs: number): Promise<boolean> {
    const until = Date.now() + withinMs;
    for (;;) {
      const [mine, theirs] = await Promise.all([s.rows(), freshRows()]);
      if (canonical(mine) === canonical(theirs)) return true;
      if (Date.now() > until) {
        record({ case: 'converge-miss', mine: mine.length, fresh: theirs.length });
        return false;
      }
      await sleep(1_000);
    }
  }

  /** One E run: live writes at `perSecond` during the transfer at `mbps`. */
  async function eRun(run: number, mbps: number, perSecond: number): Promise<void> {
    const lossBefore = await lossCount(port);
    const { s, connectedAt } = await open(mbps);
    s.send({ type: 'countPrefix', prefix: 'k-' });
    const withinMs = Math.max(120_000, (2 * 4_400_000 * 8 * 1000) / (mbps * 1e6));
    await sleep(Math.max(0, connectedAt + 3_000 - Date.now()));
    let completeAt: number | null = null;
    let bytesAtComplete = 0;
    const deadline = connectedAt + withinMs;
    const watcher = (async () => {
      await poll(() => (s.status?.prefixKeys ?? 0) >= SEED_ROWS, withinMs, 50);
      if ((s.status?.prefixKeys ?? 0) >= SEED_ROWS) {
        completeAt = Date.now();
        bytesAtComplete = proxy!.bytesDownstream();
      }
    })();
    const written = await writeAt(
      writer,
      perSecond,
      `e${perSecond}-${mbps}-${run}`,
      () => completeAt !== null || Date.now() > deadline,
    );
    await watcher;
    const converged = completeAt !== null && (await converge(s, 30_000));
    const lossAfter = await lossCount(port);
    const elapsedMs = completeAt === null ? null : completeAt - connectedAt;
    const minMs = ((0.8 * bytesAtComplete * 8) / (mbps * 1e6)) * 1000;
    const disconnects = disconnectsOf(s.status).length;
    record({
      case: 'E',
      run,
      mbps,
      perSecond,
      written,
      complete: completeAt !== null,
      converged,
      disconnects,
      lossDelta: lossAfter - lossBefore,
      bytes: bytesAtComplete,
      elapsedMs,
      minMs,
    });
    expect(disconnects).toBe(0);
    expect(completeAt).not.toBeNull();
    expect(converged).toBe(true);
    expect(lossAfter - lossBefore).toBe(0);
    expect(bytesAtComplete).toBeGreaterThanOrEqual(4_000_000);
    expect(elapsedMs!).toBeGreaterThanOrEqual(minMs);
    await close();
  }

  /** One F run: a queryOnce 8 s into the transfer at `mbps`. */
  async function fRun(run: number, mbps: number, timeoutMs: number | undefined): Promise<void> {
    const { s, connectedAt } = await open(mbps);
    s.send({ type: 'countPrefix', prefix: 'k-' });
    await sleep(Math.max(0, connectedAt + 8_000 - Date.now()));
    const reply = s.next('queryOnceResult');
    s.send({ type: 'queryOnce', map: SMALL, ...(timeoutMs === undefined ? {} : { timeoutMs }) });
    const result = (await reply) as QueryOnceResult;
    const bytesAtSettle = proxy!.bytesDownstream();
    const sockets = socketStats([port, proxy!.port]);
    const withinMs = Math.max(120_000, (2 * 4_400_000 * 8 * 1000) / (mbps * 1e6));
    const complete = await poll(() => (s.status?.prefixKeys ?? 0) >= SEED_ROWS, withinMs, 100);
    const finalBytes = proxy!.bytesDownstream();
    const d = bound(mbps);
    record({
      case: 'F',
      run,
      mbps,
      ok: result.ok,
      ms: result.ms,
      boundMs: d,
      error: result.error ?? null,
      rows: result.rows?.length ?? null,
      bytesAtSettle,
      finalBytes,
      complete,
      trace,
      sockets,
    });
    expect(complete).toBe(true);
    if (LINUX) {
      expect(result.ok).toBe(true);
      expect(result.rows).toHaveLength(SMALL_ROWS);
      expect(bytesAtSettle).toBeLessThan(0.9 * finalBytes);
      expect(result.ms).toBeLessThanOrEqual(d);
    }
    await close();
  }

  for (const mbps of RATES) {
    const evidence = mbps !== 1;
    const eRates = E_WRITES_OVERRIDE ?? (evidence ? [2, 7, 20] : [7, 20]);
    const transferMs = Math.max(120_000, (2 * 4_400_000 * 8 * 1000) / (mbps * 1e6));
    for (const perSecond of eRates) {
      test(
        `E: live writes at ${perSecond}/s during a ~4.1 MB snapshot at ${mbps} Mbit/s cause no reconnect, ${RUNS} of ${RUNS}`,
        async () => {
          for (let run = 0; run < RUNS; run++) await eRun(run, mbps, perSecond);
        },
        RUNS * (transferMs + 90_000),
      );
    }
    test(
      `F: a queryOnce during the transfer at ${mbps} Mbit/s settles before it completes, ${RUNS} of ${RUNS}`,
      async () => {
        for (let run = 0; run < RUNS; run++) await fRun(run, mbps, evidence ? 30_000 : undefined);
      },
      RUNS * (transferMs + 60_000),
    );
  }

  if (RATES.includes(1)) {
    test(
      `G: a live event of another query arrives during the transfer, ${RUNS} of ${RUNS}`,
      async () => {
        for (let run = 0; run < RUNS; run++) {
          const { s, connectedAt } = await open(1);
          s.send({ type: 'watch2', map: SMALL });
          s.send({ type: 'countPrefix', prefix: 'k-' });
          const smallReady = await poll(() => (s.status?.keys2 ?? 0) >= SMALL_ROWS + run, 60_000);
          await sleep(Math.max(0, connectedAt + 8_000 - Date.now()));
          const key = `g-${run}`;
          await writeAcked(writer, SMALL, [[key, { run }]]);
          const ackedAt = Date.now();
          let arrived: Status | null = null;
          await poll(
            () => {
              if ((s.status?.keys2 ?? 0) >= SMALL_ROWS + run + 1) arrived = s.status;
              return arrived !== null;
            },
            120_000,
            10,
          );
          const arrivedAt = arrived === null ? null : Date.now();
          const sockets = socketStats([port, proxy!.port]);
          const reply = s.next('rows');
          s.send({ type: 'dump' });
          const rows2: Array<[string, unknown]> = (await reply).rows2;
          const d = bound(1);
          const prefixAtArrival = (arrived as Status | null)?.prefixKeys ?? null;
          record({
            case: 'G',
            run,
            smallReady,
            prefixAtArrival,
            ms: arrivedAt === null ? null : arrivedAt - ackedAt,
            ackedAtMs: ackedAt - connectedAt,
            boundMs: d,
            trace,
            sockets,
          });
          expect(smallReady).toBe(true);
          expect(arrived).not.toBeNull();
          expect(rows2.some(([k]) => k === key)).toBe(true);
          if (LINUX) {
            expect(prefixAtArrival!).toBeLessThan(SEED_ROWS);
            expect(arrivedAt! - ackedAt).toBeLessThanOrEqual(d);
          }
          await close();
        }
      },
      RUNS * 180_000,
    );

    test('H: a client that stops reading mid-transfer is still disconnected by the owed-loss rule', async () => {
      const sentinel = path.join(os.tmpdir(), `overtake-h-${process.pid}-${Date.now()}`);
      const lossBefore = await lossCount(port);
      const { s, connectedAt } = await open(1);
      await sleep(Math.max(0, connectedAt + 3_000 - Date.now()));
      let stopWrites = false;
      const writes = writeAt(writer, 20, `h-${Date.now()}`, () => stopWrites);
      await sleep(Math.max(0, connectedAt + 8_000 - Date.now()));
      const unblocked = s.next('unblocked');
      s.send({ type: 'block', sentinel, maxMs: 45_000 });
      const blockedAt = Date.now();
      let lossAt: number | null = null;
      while (Date.now() < blockedAt + 30_000) {
        if ((await lossCount(port)) > lossBefore) {
          lossAt = Date.now();
          break;
        }
        await sleep(500);
      }
      await sleep(Math.max(0, blockedAt + 40_000 - Date.now()));
      stopWrites = true;
      const written = await writes;
      fs.writeFileSync(sentinel, '');
      const { found } = await unblocked;
      fs.rmSync(sentinel, { force: true });
      const reconnected = await poll(
        () =>
          (s.status?.changes ?? []).some((c) => c.to === SyncState.CONNECTED && c.at > blockedAt),
        60_000,
      );
      const converged = reconnected && (await converge(s, 120_000));
      record({
        case: 'H',
        written,
        found,
        lossMs: lossAt === null ? null : lossAt - blockedAt,
        reconnected,
        converged,
      });
      // On macOS the child's receive buffer is auto-tuned too, so a child that
      // stops reading can absorb the live traffic of the whole block.
      if (LINUX) {
        expect(lossAt).not.toBeNull();
        expect(lossAt! - blockedAt).toBeLessThanOrEqual(30_000);
        expect(reconnected).toBe(true);
        expect(converged).toBe(true);
      }
    }, 300_000);
  }
});
