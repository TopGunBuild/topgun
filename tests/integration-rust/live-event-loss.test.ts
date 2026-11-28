/**
 * Integration: a live subscriber that misses an event it is owed is
 * disconnected, reconnects, and ends with the view a fresh query gives
 * (TG-LIVE-003), on a real server.
 *
 * S is a TopGunClient in a process of its own (helpers/client-subscriber-process.js)
 * holding a live query of map `M` (no filter, sort or limit) and a live search
 * of `M` for `needle`. W is a protocol-level connection of the test that writes
 * raw OP_BATCHes of 100 ops, ~1 KiB values, keys `k0000`…`k3999`; the values of
 * `k0000`…`k0399` carry the word `needle`, no other value does. W waits for each
 * batch's OP_ACK and sends at most one batch per 10 ms, and keeps a ledger of the
 * last value per key.
 *
 * Main arm (every run on a fresh server): S blocks its event loop (`block`), so
 * it reads nothing from its socket; W writes the keys round after round until
 * the server logs the loss line (`a live event owed to a subscriber was lost`);
 * then S is released, and W rewrites every key once and removes every eighth
 * (500 keys, 50 of them `needle` keys), so ENTER, UPDATE and LEAVE are all owed
 * also after the reconnect. Once S has settled — connected again after its last
 * disconnect and after the server's last loss line, the query's and the search's resubscribe responses received, no
 * overflow pending, and its view unchanged for 1 s — its query rows must equal a
 * fresh `queryOnce` and W's ledger, and its search key set a fresh search's.
 *
 * Sensitivity arm (`LIVE_LOSS_ARM=base`, run against a build without the fix):
 * S runs an event-loop hog instead of the block, so it drains its socket
 * intermittently and the server's drops are interleaved with successful sends;
 * W writes every key once, rewrites each once and removes 500; the outcome is
 * recorded, not asserted.
 *
 * Environment:
 *
 * - RUST_SERVER_BINARY  — the server to launch (default `target/release/topgun-server`);
 * - LIVE_LOSS_RUNS      — number of runs (default 3);
 * - LIVE_LOSS_ARM       — `main` (default) or `base`;
 * - LIVE_LOSS_HOG_BUSY_MS / LIVE_LOSS_HOG_PERIOD_MS — the base arm's hog (8 / 10);
 * - LIVE_LOSS_OUT       — a JSONL file each run appends its record to;
 * - LIVE_LOSS_COMMIT    — the server's commit, written to the record;
 * - LIVE_LOSS_LOG_DIR   — a directory each run writes the server's stdout to.
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { TopGunClient, SyncState } from '@topgunbuild/client';

import { createTestToken, MemoryStorageAdapter } from './helpers';
import { TestClient } from './helpers/test-client';
import {
  connectWithin,
  firstFrame,
  plainLines,
  sleep,
  spawnBinary,
  SpawnedBinary,
} from './helpers/two-binary';

const MAP = 'live_loss_m';
const KEYS = 4_000;
const NEEDLE_KEYS = 400;
const REMOVE_EVERY = 8;
const BATCH = 100;
const BATCH_GAP_MS = 10;
const ACK_WAIT_MS = 10_000;
const STEP_WAIT_MS = 20_000;
const LOSS_CEILING_MS = 30_000;
const LOSS_CEILING_OPS = 400_000;
const SETTLE_CEILING_MS = 60_000;
const PAD = 'x'.repeat(1_000);

const LOSS_TEXT = 'a live event owed to a subscriber was lost';
const RUNS = Number(process.env.LIVE_LOSS_RUNS ?? 3);
const ARM = process.env.LIVE_LOSS_ARM === 'base' ? 'base' : 'main';
const HOG_BUSY_MS = Number(process.env.LIVE_LOSS_HOG_BUSY_MS ?? 8);
const HOG_PERIOD_MS = Number(process.env.LIVE_LOSS_HOG_PERIOD_MS ?? 10);
const REPO_ROOT = path.resolve(__dirname, '..', '..');
const BINARY =
  process.env.RUST_SERVER_BINARY ?? path.join(REPO_ROOT, 'target', 'release', 'topgun-server');

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
  overflowResults: number;
  lastOverflowAt: number | null;
  searchResults: number;
  lastSearchResultAt: number | null;
}

interface Dump {
  rows: Array<[string, unknown]>;
  searchRows: string[];
}

const key = (i: number) => `k${String(i).padStart(4, '0')}`;
const isNeedle = (i: number) => i < NEEDLE_KEYS;
const value = (i: number, round: number) => ({
  round,
  key: key(i),
  text: `${isNeedle(i) ? 'needle' : 'plain'} round ${round}`,
  pad: PAD,
});

/** A canonical form of a row value, so two views compare whatever their key order. */
function canonical(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    return `{${Object.keys(o)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`)
      .join(',')}}`;
  }
  return JSON.stringify(v);
}

/** The server's timestamp of a log line, ms since the epoch. */
function lineTime(line: string): number | null {
  const m = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z)/.exec(line.trim());
  return m ? Date.parse(m[1]) : null;
}

class Subscriber {
  readonly proc: child_process.ChildProcess;
  status: Status | null = null;
  /** Every distinct `lastServerResultAt` seen, ascending. */
  readonly serverResultTimes: number[] = [];
  private readonly waiting: Array<{ type: string; resolve: (m: unknown) => void }> = [];

  constructor(port: number) {
    this.proc = child_process.fork(
      path.join(__dirname, 'helpers', 'client-subscriber-process.js'),
      [String(port), MAP],
      { stdio: ['ignore', 'inherit', 'inherit', 'ipc'] },
    );
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- IPC messages of the subscriber process
    this.proc.on('message', (message: any) => {
      if (message?.type === 'status') {
        this.status = message as Status;
        const at = this.status.lastServerResultAt;
        if (at !== null && this.serverResultTimes[this.serverResultTimes.length - 1] !== at) {
          this.serverResultTimes.push(at);
        }
        return;
      }
      const i = this.waiting.findIndex((w) => w.type === message?.type);
      if (i >= 0) this.waiting.splice(i, 1)[0].resolve(message);
    });
  }

  send(message: Record<string, unknown>): void {
    this.proc.send(message);
  }

  reply<T>(type: string, timeoutMs: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`no ${type} from S within ${timeoutMs} ms`)),
        timeoutMs,
      );
      this.waiting.push({
        type,
        resolve: (m) => {
          clearTimeout(timer);
          resolve(m as T);
        },
      });
    });
  }

  async dump(): Promise<Dump> {
    const reply = this.reply<Dump>('rows', STEP_WAIT_MS);
    this.send({ type: 'dump' });
    return reply;
  }

  kill(): void {
    this.proc.kill('SIGKILL');
  }
}

async function until(what: string, probe: () => boolean, ceilingMs: number): Promise<void> {
  const deadline = Date.now() + ceilingMs;
  for (;;) {
    if (probe()) return;
    if (Date.now() >= deadline) throw new Error(`${what}: not within ${ceilingMs} ms`);
    await sleep(25);
  }
}

/** W: raw OP_BATCHes of at most 100 ops, each awaited, at most one per 10 ms. */
class Writer {
  readonly ledger = new Map<string, unknown>();
  ops = 0;
  private batchNo = 0;
  private lastSend = 0;

  constructor(private readonly raw: TestClient) {}

  async batch(
    items: Array<{ i: number; round: number } | { i: number; remove: true }>,
  ): Promise<void> {
    const ops = items.map((item, n) => {
      const id = `w-${this.batchNo}-${n}`;
      const timestamp = { millis: Date.now(), counter: n, nodeId: 'live-loss-writer' };
      if ('remove' in item) {
        return {
          id,
          mapName: MAP,
          opType: 'REMOVE',
          key: key(item.i),
          record: { value: null, timestamp },
        };
      }
      return {
        id,
        mapName: MAP,
        opType: 'PUT',
        key: key(item.i),
        record: { value: value(item.i, item.round), timestamp },
      };
    });
    this.batchNo += 1;
    const gap = this.lastSend + BATCH_GAP_MS - Date.now();
    if (gap > 0) await sleep(gap);
    this.raw.messages.length = 0;
    this.lastSend = Date.now();
    this.raw.send({ type: 'OP_BATCH', payload: { ops } });
    const frame = await firstFrame(this.raw, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
    const lastId = ops[ops.length - 1].id;
    if (frame?.type !== 'OP_ACK' || frame.payload?.lastId !== lastId) {
      throw new Error(
        `batch ending ${lastId}: ${JSON.stringify(frame)?.slice(0, 400) ?? 'no ack within 10 s'}`,
      );
    }
    for (const item of items) {
      if ('remove' in item) this.ledger.delete(key(item.i));
      else this.ledger.set(key(item.i), value(item.i, item.round));
    }
    this.ops += ops.length;
  }

  /** Writes every key once with `round`, in batches; `stop` is checked between batches. */
  async round(round: number, stop: () => boolean = () => false): Promise<boolean> {
    for (let at = 0; at < KEYS; at += BATCH) {
      if (stop()) return true;
      const items = [];
      for (let i = at; i < Math.min(at + BATCH, KEYS); i++) items.push({ i, round });
      await this.batch(items);
    }
    return stop();
  }

  async removeEveryEighth(): Promise<void> {
    const items: Array<{ i: number; remove: true }> = [];
    for (let i = 0; i < KEYS; i += REMOVE_EVERY) items.push({ i, remove: true });
    for (let at = 0; at < items.length; at += BATCH) await this.batch(items.slice(at, at + BATCH));
  }
}

function makeClient(port: number, userId: string): TopGunClient {
  const token = createTestToken(userId, ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
  });
}

/**
 * The last disconnect and the first CONNECTED after it. A disconnect is a
 * change into a state outside SYNCING and CONNECTED: a further AUTH_ACK on the
 * same socket (an older client's duplicate handshake ack) moves CONNECTED →
 * SYNCING → CONNECTED without losing the connection or its live query, so it
 * moves neither time. `changes` starts after the first CONNECTED, so with no
 * disconnect both are null.
 */
function lastReconnect(changes: Change[]): {
  disconnectAt: number | null;
  connectedAt: number | null;
} {
  let disconnectAt: number | null = null;
  let connectedAt: number | null = null;
  for (const c of changes) {
    if (c.to === SyncState.CONNECTED) {
      if (disconnectAt !== null) connectedAt ??= c.at;
    } else if (c.to !== SyncState.SYNCING) {
      disconnectAt = c.at;
      connectedAt = null;
    }
  }
  return { disconnectAt, connectedAt };
}

/**
 * Why S has not settled yet, or null once it has (main-arm conditions). `losses`
 * are the times of the server's loss lines: a loss after S's last CONNECTED has
 * cancelled that connection, whose socket closes seconds later, while S still
 * looks connected and its view, which the server no longer feeds, looks stable.
 */
function unsettled(s: Status | null, losses: number[]): string | null {
  if (!s) return 'no status from S';
  if (s.state !== SyncState.CONNECTED) return `S is ${s.state}`;
  const { disconnectAt, connectedAt } = lastReconnect(s.changes);
  if (disconnectAt !== null && (connectedAt === null || connectedAt < disconnectAt)) {
    return 'S has not reconnected after its last disconnect';
  }
  const since = connectedAt ?? 0;
  if (losses.some((t) => t >= since)) {
    return 'a loss after the last CONNECTED is not yet followed by a reconnect';
  }
  if (s.lastServerResultAt === null || s.lastServerResultAt < since) {
    return "the live query's resubscribe response has not arrived";
  }
  if (s.lastSearchResultAt === null || s.lastSearchResultAt < since) {
    return "the live search's resubscribe response has not arrived";
  }
  if (s.lastOverflowAt !== null && s.lastOverflowAt >= s.lastServerResultAt) {
    return 'an overflow resubscribe is pending (or the overflow is terminal)';
  }
  return null;
}

describe('Integration: a subscriber that loses an owed live event resyncs (TG-LIVE-003)', () => {
  let server: SpawnedBinary | null = null;
  let subscriber: Subscriber | null = null;
  const raws: TestClient[] = [];
  const clients: TopGunClient[] = [];

  afterEach(async () => {
    if (server && process.env.LIVE_LOSS_LOG_DIR) {
      const name = expect.getState().currentTestName?.replace(/[^a-z0-9]+/gi, '-') ?? 'run';
      fs.writeFileSync(path.join(process.env.LIVE_LOSS_LOG_DIR, `${name}.log`), server.stdout());
      fs.writeFileSync(path.join(process.env.LIVE_LOSS_LOG_DIR, `${name}.err`), server.stderr());
    }
    subscriber?.send({ type: 'exit' });
    await sleep(100);
    subscriber?.kill();
    subscriber = null;
    for (const raw of raws.splice(0)) raw.close();
    for (const client of clients.splice(0)) await client.close().catch(() => undefined);
    await server?.stop('SIGTERM', 10_000).catch(() => undefined);
    server = null;
  });

  for (let run = 1; run <= RUNS; run++) {
    test(`${ARM} arm, run ${run} of ${RUNS}`, async () => {
      if (!fs.existsSync(BINARY)) {
        throw new Error(`no server binary at ${BINARY}: build it or set RUST_SERVER_BINARY`);
      }
      // The log is read from the capture spawnBinary starts in the spawn's own
      // tick, never from a single chunk: a line can be split across chunks.
      server = await spawnBinary({ binaryPath: BINARY, label: `live-event-loss ${ARM} ${run}` });
      if (server.port === null)
        throw new Error(`the server did not start: ${server.stderr().slice(-2000)}`);
      const port = server.port;
      const log = () => plainLines(server!.stdout());
      const lossLines = () => log().filter((l) => l.includes(LOSS_TEXT));

      const s = new Subscriber(port);
      subscriber = s;
      await until(
        'S connects and holds its empty snapshot',
        () => (s.status?.serverResults ?? 0) >= 1,
        STEP_WAIT_MS,
      );
      s.send({ type: 'search', query: 'needle', limit: 1_000 });
      await until(
        'S holds its empty search result',
        () => (s.status?.searchResults ?? 0) >= 1,
        STEP_WAIT_MS,
      );

      const raw = await connectWithin(port, 'live-loss-writer', STEP_WAIT_MS);
      raws.push(raw);
      const w = new Writer(raw);

      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'live-loss-'));
      const sentinel = path.join(dir, 'release');
      let round = 0;
      let unblocked: Promise<{ found: boolean }> | null = null;
      if (ARM === 'main') {
        unblocked = s.reply<{ found: boolean }>('unblocked', LOSS_CEILING_MS + 30_000);
        s.send({ type: 'block', sentinel, maxMs: LOSS_CEILING_MS + 15_000 });
        await sleep(200);
        const started = Date.now();
        const lost = () =>
          lossLines().length > 0 ||
          Date.now() - started > LOSS_CEILING_MS ||
          w.ops >= LOSS_CEILING_OPS;
        while (!(await w.round(round, lost))) round += 1;
        fs.writeFileSync(sentinel, '');
        if (lossLines().length === 0) throw new Error('forced loss not reached');
        expect((await unblocked).found).toBe(true);
      } else {
        s.send({ type: 'hog', busyMs: HOG_BUSY_MS, periodMs: HOG_PERIOD_MS });
        await w.round(round);
      }
      round += 1;
      await w.round(round);
      await w.removeEveryEighth();
      if (ARM === 'base') s.send({ type: 'hog', off: true });
      const lastAck = Date.now();

      // Settle.
      const disconnected = () =>
        (s.status?.changes ?? []).some((c) => c.from === SyncState.CONNECTED);
      let dump: Dump | null = null;
      let stableFor = 0;
      for (;;) {
        const lossTimes = lossLines()
          .map(lineTime)
          .filter((t): t is number => t !== null);
        const why = ARM === 'main' || disconnected() ? unsettled(s.status, lossTimes) : null;
        if (why === null) {
          const next = await s.dump();
          if (dump && canonical(next) === canonical(dump)) stableFor += 250;
          else stableFor = 0;
          dump = next;
          const need = ARM === 'main' || disconnected() ? 1_000 : 2_000;
          if (stableFor >= need) break;
        } else {
          stableFor = 0;
        }
        if (Date.now() - lastAck > SETTLE_CEILING_MS) {
          throw new Error(
            `S did not settle within ${SETTLE_CEILING_MS} ms: ${why ?? 'its view kept changing'}; ` +
              `${lossLines().length} loss lines, last state changes ` +
              JSON.stringify((s.status?.changes ?? []).slice(-6)),
          );
        }
        await sleep(250);
      }

      // A fresh query and a fresh search from a new client.
      const fresh = makeClient(port, 'live-loss-fresh');
      clients.push(fresh);
      await fresh.start();
      await until(
        'the fresh client connects',
        () => fresh.getConnectionState() === SyncState.CONNECTED,
        STEP_WAIT_MS,
      );
      const freshRows = await fresh.queryOnce(MAP, {}, { timeoutMs: STEP_WAIT_MS });
      const freshMap = new Map<string, string>();
      for (const row of freshRows) {
        const { _key, ...v } = row as Record<string, unknown>;
        freshMap.set(String(_key), canonical(v));
      }
      const freshSearch = (await fresh.search(MAP, 'needle', { limit: 1_000 }))
        .map((r) => r.key)
        .sort();

      const sMap = new Map(dump!.rows.map(([k, v]) => [k, canonical(v)]));
      const ledgerMap = new Map([...w.ledger].map(([k, v]) => [k, canonical(v)]));
      const sameMap = (a: Map<string, string>, b: Map<string, string>) =>
        a.size === b.size && [...a].every(([k, v]) => b.get(k) === v);
      const queryEqual = sameMap(sMap, freshMap);
      const freshEqualsLedger = sameMap(freshMap, ledgerMap);
      const ledgerNeedles = [...w.ledger.keys()]
        .filter((k) => Number(k.slice(1)) < NEEDLE_KEYS)
        .sort();
      const searchEqual = canonical(dump!.searchRows) === canonical(freshSearch);
      const searchEqualsLedger = canonical(dump!.searchRows) === canonical(ledgerNeedles);

      // Cycles: each loss line, the CONNECTED after it, and the first server result from then on.
      const status = s.status!;
      const losses = lossLines()
        .map(lineTime)
        .filter((t): t is number => t !== null);
      const cycles = losses.map((tLoss) => {
        const conn = status.changes.find((c) => c.to === SyncState.CONNECTED && c.at > tLoss);
        const resp = conn ? s.serverResultTimes.find((t) => t >= conn.at) : undefined;
        return {
          tLoss,
          respMinusLoss: resp !== undefined ? resp - tLoss : null,
          respMinusConn: resp !== undefined && conn ? resp - conn.at : null,
        };
      });
      const firstLoss = losses[0];
      const exercised =
        firstLoss !== undefined &&
        status.changes.some((c) => c.from === SyncState.CONNECTED && c.at > firstLoss) &&
        status.changes.some((c) => c.to === SyncState.CONNECTED && c.at > firstLoss);

      const record = {
        arm: ARM,
        run,
        commit: process.env.LIVE_LOSS_COMMIT ?? null,
        sha256: server.sha256,
        hog: ARM === 'base' ? { busyMs: HOG_BUSY_MS, periodMs: HOG_PERIOD_MS } : null,
        opsWritten: w.ops,
        lossesSeen: losses.length,
        cycles,
        reconnects: status.changes.filter((c) => c.to === SyncState.CONNECTED).length,
        overflows: status.overflowResults,
        sQueryKeys: sMap.size,
        freshQueryKeys: freshMap.size,
        queryEqual,
        freshEqualsLedger,
        sSearchKeys: dump!.searchRows.length,
        freshSearchKeys: freshSearch.length,
        searchEqual,
        searchEqualsLedger,
        exercised,
      };
      if (process.env.LIVE_LOSS_OUT)
        fs.appendFileSync(process.env.LIVE_LOSS_OUT, `${JSON.stringify(record)}\n`);
      process.stderr.write(
        `[live-event-loss] ${JSON.stringify({ ...record, cycles: cycles.length })}\n`,
      );
      fs.rmSync(dir, { recursive: true, force: true });

      if (ARM === 'main') {
        expect(exercised).toBe(true);
        expect(freshMap.size).toBe(KEYS - KEYS / REMOVE_EVERY);
        expect(freshEqualsLedger).toBe(true);
        expect(queryEqual).toBe(true);
        expect(freshSearch.length).toBeGreaterThan(0);
        expect(searchEqual).toBe(true);
      }
    }, 240_000);
  }
});
