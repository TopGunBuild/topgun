/**
 * Integration: a server whose RocksDB store reports an I/O error stops in
 * order with exit code 72, and a restart on a writable directory reads back
 * every acknowledged write.
 *
 * Once RocksDB has met an I/O error on a write it refuses every later write
 * on the same handle (its background error); only a fresh handle — a new
 * process — can store again. A write is acknowledged once it is in the
 * write-ahead log, so the acknowledged writes the broken handle could not
 * store are replayed from the WAL by the next process. This test makes the
 * handle break under a running server and checks what a process supervisor
 * and an operator would see: the process ends by itself with code 72, the
 * store and the binary say why, readiness stops answering 200, and after a
 * restart every acknowledged write is readable (`TG-WB-010`).
 *
 * The I/O error is real. The engine directory `engine.rocksdb` is made
 * read-only (mode 0555 on the directory and its files) under the running
 * server. Files RocksDB already holds open stay writable, so the error comes
 * when RocksDB must create a file: a memtable switch opens a new engine log
 * and a flush a new table file. The server runs with
 * `TOPGUN_ENGINE_CACHE_MB=64` (8 MiB memtables), and the test writes large
 * late values until the store reports broken, bounded by 32 MiB of values.
 * RocksDB carries no subcode for the error, so the store names it
 * `cause="io"` with `io_kind="Other"`.
 *
 * The binary is the release binary of this checkout: RUST_SERVER_BINARY when
 * set, else the one `cargo build --release --bin topgun-server` produces. The
 * test fails, never skips, when run as root: root writes into a read-only
 * directory, so no I/O error would come.
 *
 * Two arms, one data directory each:
 *
 * - LATCH-THEN-ORDERLY-EXIT — a few keys are written and reach the store, the
 *   engine directory is made read-only, and late keys are written until the
 *   store reports broken. The process exits 72 by itself inside the server's
 *   own deadline for a failed stop. The modes are restored and a restart on
 *   the same directory reads exactly the acknowledged keys.
 * - STILL-READ-ONLY-AT-RESTART — the same up to the exit 72, but the modes
 *   stay read-only for a restart: opening RocksDB for writing must open its
 *   LOCK file for writing and create a new engine log, so the restarted
 *   process exits 1 at the store open before it is ready, names the
 *   permission error, and leaves the WAL directory as it was. Then the modes
 *   are restored and a third start reads exactly the acknowledged keys.
 *
 * Each arm checks that its own setup held (its precondition) before it says
 * anything. A precondition that did not hold is an error of the scenario — the
 * run is neither a pass nor a failure of the arm — and is thrown as
 * `ScenarioPreconditionError`, whose message starts with
 * `SCENARIO PRECONDITION NOT MET`. Repeat such a run; do not loosen the check.
 *
 * Environment:
 *
 * - RUST_SERVER_BINARY        — the release binary to run (else built here);
 * - STORE_BROKEN_EXIT_WAIT_MS — how long the test waits for the process to end
 *                               by itself (default: the server's own deadline
 *                               for a failed stop plus a margin, 50 s);
 * - LIVENESS_OUT_DIR          — where each run writes its notes and the server
 *                               streams; unset, nothing is written;
 * - LIVENESS_RUN              — label of the run in those file names.
 */

import * as child_process from 'child_process';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { LagScrape, allZero, lwwPut, scrapeGauge, scrapeLag } from './helpers/fault-binary';
import { TestClient } from './helpers/test-client';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  sha256OfFile,
  sleep,
  plainLines,
  within,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

type Arm = 'LATCH-THEN-ORDERLY-EXIT' | 'STILL-READ-ONLY-AT-RESTART';

const REPO_ROOT = path.resolve(__dirname, '..', '..');

/** Written and flushed while the store is healthy. */
const EARLY_MAP = 'store_broken_early';
const EARLY_KEYS = ['early-0', 'early-1', 'early-2', 'early-3', 'early-4'];
/**
 * The late writes, one of whose flushes meets the I/O error. Each late key
 * has a map of its own, so the read-back of one is a small snapshot: all of
 * them together are tens of MiB.
 */
const LATE_MAP_PREFIX = 'store_broken_late_';
const lateMap = (key: string) => `${LATE_MAP_PREFIX}${key}`;
/** Each late value: large, so few writes fill a memtable. */
const LATE_PAD = 'x'.repeat(64 * 1024);
/**
 * Four memtables of 8 MiB: past this much late data without an I/O error,
 * the engine is not refusing what it must create.
 */
const LATE_BYTES_BOUND = 4 * 8 * 1024 * 1024;
/** Given after the bound, for the last flush passes to reach the engine. */
const LATE_SETTLE_MS = 10_000;

/** The store's line when it classifies an error as a broken store. */
const STORE_BROKEN_TEXT =
  'write-behind store: the durable store is broken and stores nothing more in this process; writes are refused from now on and the node stops';
/** The store's line when its shutdown drain stops at a broken store. */
const DRAIN_STOPPED_TEXT =
  'write-behind shutdown drain stopped at a broken store: the entries it did not store stay un-resolved and replay from the WAL at the next start';
/** The binary's line when it learns that the store is broken; it precedes the 503. */
const BINARY_BROKEN_TEXT = 'the durable store is broken; shutting the node down';
/** The store's line for a panicked background task: a broken store must not be reported as one. */
const PANIC_TEXT = 'write-behind background task panicked';
/** The binary's line when the store does not open. */
const STORE_OPEN_REFUSED_TEXT = 'store open: refusing to start';
/** What RocksDB says when the read-only directory refuses it. */
const PERMISSION_TEXT = 'Permission denied';
/** What a full disk would be named by: a permission error must not read as one. */
const ENOSPC_TEXT = 'No space left on device';
const BROKEN_COUNTER = 'topgun_write_behind_store_broken_total{cause="io"}';

/** Given to the server explicitly, so the deadline below is a known number. */
const SHUTDOWN_TIMEOUT_MS = 5_000;
/**
 * The server's own bound on a stop that follows a store failure: its 30 s
 * wait for in-flight requests, the shutdown timeout above, and 10 s on top.
 */
const FAILURE_DEADLINE_MS = 30_000 + SHUTDOWN_TIMEOUT_MS + 10_000;
/** Waited beyond the deadline, so that a process the deadline ended is seen as that and not as a hang. */
const EXIT_WAIT_MARGIN_MS = 5_000;

const ACK_WAIT_MS = 5_000;
const CONNECT_WAIT_MS = 5_000;
const SCRAPE_WAIT_MS = 2_000;
/**
 * How long after the last acknowledgement the early writes had all reached the
 * store on the real binary: write delay, one flush tick, and the 600 ms tick
 * that refreshes the gauge.
 */
const FLUSH_OBSERVED_MS = 2_400;
/** Room on top of that for a slower host; the wait ends as soon as the sign is seen. */
const FLUSH_MARGIN_MS = 3_600;
const FLUSH_POLL_MS = 100;
const READY_POLL_MS = 20;
/** How long a restart on a still read-only directory may take to end by itself once it has not printed its port. */
const STILL_READ_ONLY_EXIT_WAIT_MS = 15_000;
const RESTART_STOP_MS = 15_000;
/** Bounds a hung arm: the build, boot, the late writes, the wait for the exit, and up to two restarts. */
const ARM_TIMEOUT_MS = 600_000;

/**
 * The setup of an arm did not hold, so the arm has no result: not a pass and
 * not a failure. Kept apart from a failed assertion by its name and by the
 * first words of its message.
 */
class ScenarioPreconditionError extends Error {
  constructor(arm: Arm, what: string) {
    super(
      `SCENARIO PRECONDITION NOT MET (arm ${arm}): ${what}. This run is neither a pass nor a failure of the arm: repeat it, and do not loosen the precondition.`,
    );
    this.name = 'ScenarioPreconditionError';
  }
}

function exitWaitMs(): number {
  const raw = process.env.STORE_BROKEN_EXIT_WAIT_MS;
  if (!raw) return FAILURE_DEADLINE_MS + EXIT_WAIT_MARGIN_MS;
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(
      `STORE_BROKEN_EXIT_WAIT_MS is not a positive whole number of milliseconds: ${raw}`,
    );
  }
  return value;
}

/** The release binary: RUST_SERVER_BINARY, else built here. */
function releaseBinary(): { binary: string; commit: string } {
  const commit = child_process
    .execFileSync('git', ['rev-parse', 'HEAD'], { cwd: REPO_ROOT })
    .toString()
    .trim();
  const named = process.env.RUST_SERVER_BINARY;
  if (named) return { binary: named, commit };
  const out = child_process.execFileSync(
    'cargo',
    ['build', '--release', '--bin', 'topgun-server', '--message-format=json'],
    { cwd: REPO_ROOT, stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 256 * 1024 * 1024 },
  );
  for (const line of out.toString().split('\n').reverse()) {
    if (!line.includes('"executable"')) continue;
    const message = JSON.parse(line);
    if (message.target?.name === 'topgun-server' && message.executable) {
      return { binary: message.executable as string, commit };
    }
  }
  throw new Error('cargo build --release --bin topgun-server named no executable');
}

/**
 * Fails — never skips — when a read-only directory accepts a write: under root
 * it does, and no I/O error can come.
 */
function requireReadOnlyRefuses(scratch: string): void {
  const probe = path.join(scratch, 'read-only-probe');
  fs.mkdirSync(probe);
  fs.chmodSync(probe, 0o555);
  let refused = false;
  try {
    fs.writeFileSync(path.join(probe, 'file'), '');
  } catch {
    refused = true;
  }
  fs.chmodSync(probe, 0o755);
  fs.rmSync(probe, { recursive: true, force: true });
  if (!refused) {
    throw new Error('running as root: the read-only directory cannot produce an IOError');
  }
}

/** Sets `mode` on `dir` and on every entry under it. */
function chmodTree(dir: string, mode: number): void {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) chmodTree(entryPath, mode);
    else fs.chmodSync(entryPath, mode);
  }
  fs.chmodSync(dir, mode);
}

/** One hash over the names, sizes and contents of every file under `dir`. */
function treeDigest(dir: string): string {
  const hash = crypto.createHash('sha256');
  const walk = (at: string) => {
    for (const entry of fs
      .readdirSync(at, { withFileTypes: true })
      .sort((a, b) => a.name.localeCompare(b.name))) {
      const entryPath = path.join(at, entry.name);
      hash.update(path.relative(dir, entryPath));
      if (entry.isDirectory()) walk(entryPath);
      else hash.update(fs.readFileSync(entryPath));
    }
  };
  walk(dir);
  return hash.digest('hex');
}

/** Sends one write and reports the type of the frame that answered it, or what happened instead. */
async function writeAndSee(
  client: TestClient,
  id: string,
  mapName: string,
  key: string,
  value: unknown,
): Promise<string> {
  const ws = client.ws;
  if (ws.readyState !== ws.OPEN) return 'not sent: the connection was already closed';
  client.messages.length = 0;
  client.send({ type: 'CLIENT_OP', payload: lwwPut(id, mapName, key, value) });
  const deadline = Date.now() + ACK_WAIT_MS;
  for (;;) {
    // A frame that arrived together with the close is still the answer.
    const frame = client.messages.find((m) => ['OP_ACK', 'OP_REJECTED', 'ERROR'].includes(m.type));
    if (frame) return frame.type as string;
    if (ws.readyState !== ws.OPEN) return 'connection closed';
    if (Date.now() >= deadline) return `no frame within ${ACK_WAIT_MS} ms`;
    await sleep(5);
  }
}

/** Reads every row of a map through one snapshot query on an open connection: key to value. */
async function readMap(client: TestClient, mapName: string): Promise<Record<string, unknown>> {
  client.messages.length = 0;
  client.send({ type: 'QUERY_SUB', payload: { queryId: `q-${mapName}`, mapName, query: {} } });
  const response = await firstFrame(client, ['QUERY_RESP'], ACK_WAIT_MS);
  if (!response) throw new Error(`no QUERY_RESP for ${mapName} in ${ACK_WAIT_MS} ms`);
  const rows: Record<string, unknown> = {};
  for (const row of response.payload.results as any[]) {
    rows[row.key] = row.record?.value ?? row.value;
  }
  return rows;
}

function linesWith(capture: string, text: string): string[] {
  return plainLines(capture).filter((line) => line.includes(text));
}

/** The status `/health/ready` answers, or `refused` when nothing answers. */
async function readiness(port: number): Promise<string> {
  try {
    const resp = await within(
      fetch(`http://127.0.0.1:${port}/health/ready`),
      SCRAPE_WAIT_MS,
      'readiness probe',
    );
    return String(resp.status);
  } catch {
    return 'refused';
  }
}

interface Observed {
  arm: Arm;
  /** Why the arm has no result, or `null` when its setup held. */
  preconditionNotMet: string | null;
  /** Late key to the answer its write got, in the order sent. */
  lateAnswers: Array<[string, string]>;
  lateBytesSent: number;
  /** How the process ended by itself, or `null` when it was still running when the wait ran out. */
  exit: BinaryExit | null;
  exitWaitMs: number;
  /** From the directory being made read-only to the process being gone. */
  exitAfterReadOnlyMs: number | null;
  storeBrokenLines: string[];
  drainStoppedLines: string[];
  binaryBrokenLines: string[];
  panicLines: number;
  /** The broken-store counter, scraped once the store's line was seen. */
  brokenCounter: number | null;
  /** The distinct readiness answers of probes started after the binary's line. */
  readinessAfterBroken: string[];
  /** Arm 2 only: the restart with the directory still read-only. */
  stillReadOnly: {
    port: number | null;
    exit: BinaryExit | null;
    refusalAtStoreOpen: boolean;
    permissionNamed: boolean;
    diskFullNamed: boolean;
    walUnchanged: boolean;
  } | null;
  /** Key to the last acknowledged value, per map. */
  acked: Record<string, Record<string, unknown>>;
  /** Every row the healed restart returned for the written maps. */
  readBack: Record<string, Record<string, unknown>> | string;
  healedExitOnSigterm: BinaryExit | null;
  sha256Before: string;
  sha256After: string;
}

async function runArm(arm: Arm): Promise<Observed> {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-store-broken-'));
  requireReadOnlyRefuses(dataDir);
  const env = releaseBinary();
  const waitForExitMs = exitWaitMs();
  const outDir = process.env.LIVENESS_OUT_DIR || null;
  if (outDir) fs.mkdirSync(outDir, { recursive: true });
  const run =
    process.env.LIVENESS_RUN ||
    new Date()
      .toISOString()
      .replace(/[^0-9]/g, '')
      .slice(0, 14);
  const storeDir = path.join(dataDir, 'data');
  const engineDir = path.join(storeDir, 'engine.rocksdb');
  const walDir = path.join(dataDir, 'wal');
  const startedAt = Date.now();
  const notes: string[] = [];
  const note = (text: string) => {
    notes.push(`${new Date().toISOString()} +${Date.now() - startedAt}ms ${text}`);
  };
  let server: SpawnedBinary | null = null;
  let stillReadOnlyServer: SpawnedBinary | null = null;
  let healed: SpawnedBinary | null = null;
  let writer: TestClient | null = null;
  let reader: TestClient | null = null;
  let summary = `store-broken arm=${arm} commit=${env.commit} aborted=before the server was launched`;

  const storeEnv: Record<string, string> = {
    // Named, because the launcher's default is the null store.
    STORAGE_BACKEND: 'rocksdb',
    TOPGUN_DATA_DIR: storeDir,
    TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
    TOPGUN_WAL_DIR: walDir,
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_ENGINE_CACHE_MB: '64',
    TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS: String(SHUTDOWN_TIMEOUT_MS),
    // Refreshes the lag gauge every 600 ms, which is what the flush wait reads.
    TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '6000',
  };
  const spawnServer = async (label: string) => {
    const proc = await spawnBinary({
      binaryPath: env.binary,
      commit: env.commit,
      label: `store-broken ${arm} ${label}`,
      env: storeEnv,
    });
    return proc;
  };

  try {
    server = await spawnServer('first start');
    const launched = server;
    const sha256Before = launched.sha256;
    const abort = (reason: string): never => {
      summary = `store-broken arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=${reason}`;
      throw new Error(
        `${reason}\n--- stdout ---\n${launched.stdout()}\n--- stderr ---\n${launched.stderr()}`,
      );
    };
    summary = `store-broken arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=the run did not reach its end`;
    if (launched.port === null) abort('the server did not print PORT=');
    const port = launched.port as number;
    note(`PORT=${port}`);
    if (!fs.existsSync(engineDir)) abort(`the server created no ${engineDir}`);

    const acked: Observed['acked'] = {};
    let preconditionNotMet: string | null = null;

    // One connection for the whole arm, kept open until the process has
    // ended: a server that waited for its clients to leave would not exit.
    writer = await connectWithin(port, 'store-broken-writer', CONNECT_WAIT_MS);

    // Step 1: several distinct keys, each acknowledged.
    for (const [i, key] of EARLY_KEYS.entries()) {
      const answer = await writeAndSee(writer, `early-${i}`, EARLY_MAP, key, { early: i });
      if (answer !== 'OP_ACK')
        abort(`step 1: the write to ${key} was answered with ${answer}, not OP_ACK`);
      (acked[EARLY_MAP] ??= {})[key] = { early: i };
    }
    const lastEarlyAckAt = Date.now();
    note(`step 1: ${EARLY_KEYS.length} early writes acknowledged`);

    // Step 2: wait until they are in the store. The sign is the lag gauge:
    // first a scrape with a sample of 1 or more — taken after the last
    // acknowledgement, so a zero from before the writes cannot pass — and
    // after it a scrape with at least one sample and every sample at zero.
    let riseAfterMs: number | null = null;
    let flushedAfterMs: number | null = null;
    let lastScrape: LagScrape = {};
    const flushDeadline = lastEarlyAckAt + FLUSH_OBSERVED_MS + FLUSH_MARGIN_MS;
    while (flushedAfterMs === null && Date.now() < flushDeadline) {
      lastScrape = await within(
        scrapeLag(port),
        SCRAPE_WAIT_MS,
        `scrape of /metrics on port ${port}`,
      );
      const at = Date.now() - lastEarlyAckAt;
      if (riseAfterMs === null && Object.values(lastScrape).some((lag) => lag >= 1)) {
        riseAfterMs = at;
      } else if (riseAfterMs !== null && allZero(lastScrape)) {
        flushedAfterMs = at;
      }
      if (flushedAfterMs === null) await sleep(FLUSH_POLL_MS);
    }
    note(
      `step 2: lag rose after ${riseAfterMs} ms, all zero after ${flushedAfterMs} ms; last scrape ${JSON.stringify(lastScrape)}`,
    );
    if (flushedAfterMs === null) {
      preconditionNotMet = `step 2: no scrape with a lag of 1 or more followed by a scrape with every lag at 0 within ${FLUSH_OBSERVED_MS + FLUSH_MARGIN_MS} ms of the last early acknowledgement (rise after ${riseAfterMs} ms, last scrape ${JSON.stringify(lastScrape)}), so the early writes are not known to be in the store`;
    }

    // Step 3: from here on RocksDB cannot create a file in its directory.
    chmodTree(engineDir, 0o555);
    const readOnlyAt = Date.now();
    note('step 3: the engine directory is read-only');

    // Readiness, probed until the process is gone; only probes started after
    // the binary's line count, since the 503 follows that line.
    const readinessAfterBroken = new Set<string>();
    // The broken-store counter, from scrapes started after the store's line
    // that were answered; `null` while none was.
    let brokenCounter: number | null = null;
    let probing = true;
    const prober = (async () => {
      while (probing) {
        const binaryLineSeen = linesWith(launched.stdout(), BINARY_BROKEN_TEXT).length > 0;
        const storeLineSeen = linesWith(launched.stdout(), STORE_BROKEN_TEXT).length > 0;
        const answer = await readiness(port);
        if (binaryLineSeen) readinessAfterBroken.add(answer);
        if (storeLineSeen) {
          try {
            const value = await within(scrapeGauge(port, BROKEN_COUNTER), SCRAPE_WAIT_MS, 'scrape');
            if (value !== null) brokenCounter = Math.max(brokenCounter ?? 0, value);
          } catch {
            // Nothing answers once the listener has closed.
          }
        }
        await sleep(READY_POLL_MS);
      }
    })();

    // Step 4: late keys until the store reports broken. Each WAL frame is
    // written before its ack; a flush to RocksDB is what meets the error.
    const lateAnswers: Array<[string, string]> = [];
    let lateBytesSent = 0;
    const brokenSeen = () => linesWith(launched.stdout(), STORE_BROKEN_TEXT).length > 0;
    if (preconditionNotMet === null) {
      let i = 0;
      let settleUntil: number | null = null;
      for (;;) {
        if (brokenSeen()) break;
        if (lateBytesSent >= LATE_BYTES_BOUND) {
          settleUntil ??= Date.now() + LATE_SETTLE_MS;
          if (Date.now() >= settleUntil) break;
          await sleep(100);
          continue;
        }
        i += 1;
        const key = `late-${String(i).padStart(4, '0')}`;
        const value = { late: i, pad: LATE_PAD };
        const answer = await writeAndSee(writer, key, lateMap(key), key, value);
        lateAnswers.push([key, answer]);
        lateBytesSent += LATE_PAD.length;
        if (answer === 'OP_ACK') acked[lateMap(key)] = { [key]: value };
        else break;
      }
      // One write after the store said it is broken, which it must refuse
      // before the WAL.
      if (brokenSeen() && lateAnswers.every(([, answer]) => answer === 'OP_ACK')) {
        i += 1;
        const key = `late-${String(i).padStart(4, '0')}`;
        const value = { late: i, pad: LATE_PAD };
        const answer = await writeAndSee(writer, key, lateMap(key), key, value);
        lateAnswers.push([key, answer]);
        if (answer === 'OP_ACK') acked[lateMap(key)] = { [key]: value };
      }
      note(
        `step 4: ${lateAnswers.length} late writes, ${lateBytesSent} bytes; answers ${JSON.stringify(
          lateAnswers.filter(([, answer]) => answer !== 'OP_ACK'),
        )}; counter ${brokenCounter}`,
      );
    }

    // Step 5: the process ends by itself, with the client connection still open.
    const exit =
      preconditionNotMet === null ? await launched.waitForExit(waitForExitMs) : launched.exit();
    probing = false;
    await prober;
    const exitAt = Date.now();
    const exitAfterReadOnlyMs = exit !== null ? exitAt - readOnlyAt : null;
    note(
      `step 5: exit ${JSON.stringify(exit)} ${exitAfterReadOnlyMs} ms after the directory became read-only`,
    );
    if (exit === null) await launched.stop('SIGKILL', 5_000);

    const out = launched.stdout();
    const storeBrokenLines = linesWith(out, STORE_BROKEN_TEXT);
    const drainStoppedLines = linesWith(out, DRAIN_STOPPED_TEXT);
    const binaryBrokenLines = linesWith(out, BINARY_BROKEN_TEXT);
    const panicLines = linesWith(out, PANIC_TEXT).length;
    for (const line of [...storeBrokenLines, ...drainStoppedLines, ...binaryBrokenLines]) {
      note(`server: ${line}`);
    }
    note(`readiness after the binary's line: ${JSON.stringify([...readinessAfterBroken])}`);
    if (writer.ws.readyState !== writer.ws.CLOSED) writer.ws.terminate();

    let stillReadOnly: Observed['stillReadOnly'] = null;
    let readBack: Observed['readBack'] = 'NOT OBSERVED: the process did not end by itself';
    let healedExitOnSigterm: BinaryExit | null = null;
    if (exit !== null) {
      if (arm === 'STILL-READ-ONLY-AT-RESTART') {
        // The modes stay: opening RocksDB for writing must create files, and cannot.
        const walBefore = treeDigest(walDir);
        stillReadOnlyServer = await spawnServer('restart on the read-only directory');
        const stillPort = stillReadOnlyServer.port;
        const stillExit =
          stillPort === null
            ? await stillReadOnlyServer.waitForExit(STILL_READ_ONLY_EXIT_WAIT_MS)
            : stillReadOnlyServer.exit();
        await stillReadOnlyServer.stop('SIGKILL', 5_000);
        const output = `${stillReadOnlyServer.stdout()}\n${stillReadOnlyServer.stderr()}`;
        const refusal = linesWith(output, STORE_OPEN_REFUSED_TEXT);
        stillReadOnly = {
          port: stillPort,
          exit: stillExit,
          refusalAtStoreOpen: refusal.length > 0,
          permissionNamed: refusal.some((line) => line.includes(PERMISSION_TEXT)),
          diskFullNamed: output.includes(ENOSPC_TEXT),
          walUnchanged: treeDigest(walDir) === walBefore,
        };
        note(`restart on the read-only directory: ${JSON.stringify(stillReadOnly)}`);
        for (const line of refusal) note(`restart on the read-only directory: ${line}`);
        if (stillPort !== null || stillExit === null || stillExit.code !== 1) {
          preconditionNotMet = `the restart on the read-only directory did not exit 1 by itself before it was ready (port ${stillPort}, exit ${JSON.stringify(stillExit)})`;
        }
      }

      // The directory is writable again.
      chmodTree(engineDir, 0o755);
      healed = await spawnServer('healed restart');
      if (healed.port === null) {
        readBack = `NOT OBSERVED: the healed restart did not print PORT= (exit ${JSON.stringify(healed.exit())}): ${healed.stderr().slice(0, 500)}`;
      } else {
        try {
          reader = await connectWithin(healed.port, 'store-broken-reader', CONNECT_WAIT_MS);
          const read: Record<string, Record<string, unknown>> = {};
          // Every map written to, acknowledged or not: a map of a key that was
          // not acknowledged must read back empty.
          for (const mapName of [EARLY_MAP, ...lateAnswers.map(([key]) => lateMap(key))]) {
            read[mapName] = await readMap(reader, mapName);
          }
          readBack = read;
        } catch (err) {
          readBack = `NOT OBSERVED: ${(err as Error).message}`;
        }
        if (reader) reader.ws.terminate();
        healedExitOnSigterm = await healed.stop('SIGTERM', RESTART_STOP_MS);
      }
      note(
        `healed restart: read back ${
          typeof readBack === 'string'
            ? readBack
            : Object.entries(readBack)
                .map(([mapName, rows]) => `${mapName}:${Object.keys(rows).length} keys`)
                .join(' ')
        }; exit on SIGTERM ${JSON.stringify(healedExitOnSigterm)}`,
      );
    }

    const sha256After = sha256OfFile(env.binary);
    const readBackOk =
      typeof readBack !== 'string' &&
      JSON.stringify(lateReadBack({ readBack, lateAnswers })) ===
        JSON.stringify(lateAcked({ acked })) &&
      JSON.stringify(readBack[EARLY_MAP]) === JSON.stringify(acked[EARLY_MAP]);
    summary =
      `store-broken arm=${arm} sha256_before=${sha256Before} sha256_after=${sha256After} commit=${env.commit} ` +
      `precondition=${preconditionNotMet === null ? 'met' : 'NOT MET'} flushed_after_ms=${flushedAfterMs} ` +
      `late_writes=${lateAnswers.length} late_bytes=${lateBytesSent} ` +
      `late_not_acked=${JSON.stringify(lateAnswers.filter(([, a]) => a !== 'OP_ACK'))} ` +
      `exit=${JSON.stringify(exit)} exit_after_read_only_ms=${exitAfterReadOnlyMs} ` +
      `store_broken_lines=${storeBrokenLines.length} drain_stopped_lines=${drainStoppedLines.length} ` +
      `binary_broken_lines=${binaryBrokenLines.length} panic_lines=${panicLines} ` +
      `broken_counter=${brokenCounter} readiness_after_broken=${JSON.stringify([...readinessAfterBroken])} ` +
      `still_read_only_restart=${JSON.stringify(stillReadOnly)} ` +
      `acked_keys=${Object.values(acked).reduce((sum, rows) => sum + Object.keys(rows).length, 0)} ` +
      `read_back=${readBackOk ? 'ok' : 'fail'} healed_exit_on_sigterm=${JSON.stringify(healedExitOnSigterm)}`;

    return {
      arm,
      preconditionNotMet,
      lateAnswers,
      lateBytesSent,
      exit,
      exitWaitMs: waitForExitMs,
      exitAfterReadOnlyMs,
      storeBrokenLines,
      drainStoppedLines,
      binaryBrokenLines,
      panicLines,
      brokenCounter,
      readinessAfterBroken: [...readinessAfterBroken].sort(),
      stillReadOnly,
      acked,
      readBack,
      healedExitOnSigterm,
      sha256Before,
      sha256After,
    };
  } finally {
    for (const client of [writer, reader]) {
      if (client && client.ws.readyState !== client.ws.CLOSED) client.ws.terminate();
    }
    for (const proc of [server, stillReadOnlyServer, healed]) {
      if (proc) await proc.stop('SIGKILL', 5_000);
    }
    // Straight to stderr: the suite silences console.log, and this line is the record of the run.
    process.stderr.write(`${summary}\n`);
    if (outDir) {
      const base = path.join(outDir, `store-broken-${arm}-${run}`);
      fs.writeFileSync(`${base}.notes.txt`, `${notes.join('\n')}\n${summary}\n`);
      fs.writeFileSync(`${base}.stdout.txt`, server ? server.stdout() : '');
      fs.writeFileSync(`${base}.stderr.txt`, server ? server.stderr() : '');
      fs.writeFileSync(
        `${base}.still-read-only.txt`,
        stillReadOnlyServer
          ? `${stillReadOnlyServer.stdout()}\n${stillReadOnlyServer.stderr()}`
          : '',
      );
      fs.writeFileSync(`${base}.healed.stdout.txt`, healed ? healed.stdout() : '');
      fs.writeFileSync(`${base}.healed.stderr.txt`, healed ? healed.stderr() : '');
    }
    if (fs.existsSync(engineDir)) chmodTree(engineDir, 0o755);
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

/** Fails the arm with a named observation when the process had to be ended by the test. */
function requireExitByItself(o: Observed): void {
  if (o.exit === null) {
    const notAcked = o.lateAnswers.filter(([, a]) => a !== 'OP_ACK');
    const reason =
      o.storeBrokenLines.length === 0 && o.lateBytesSent >= LATE_BYTES_BOUND
        ? `no IOError after ${LATE_BYTES_BOUND / (1024 * 1024)} MiB`
        : 'the store did not report broken';
    throw new Error(
      `arm ${o.arm}: ${reason}; the process did not exit by itself within ${o.exitWaitMs} ms of its engine directory being made read-only, with the client connection open (late writes: ${o.lateAnswers.length}, ${o.lateBytesSent} bytes, not acknowledged: ${JSON.stringify(notAcked)}; store's line seen: ${o.storeBrokenLines.length}, drain line seen: ${o.drainStoppedLines.length}, binary's line seen: ${o.binaryBrokenLines.length}). It was killed by the test.`,
    );
  }
}

/** Every late key acknowledged, with its value. */
function lateAcked(o: Pick<Observed, 'acked'>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [mapName, rows] of Object.entries(o.acked)) {
    if (mapName.startsWith(LATE_MAP_PREFIX)) Object.assign(out, rows);
  }
  return out;
}

/** Every late key the healed restart served, with its value, over the maps of every late write sent. */
function lateReadBack(
  o: Pick<Observed, 'readBack' | 'lateAnswers'>,
): Record<string, unknown> | string {
  if (typeof o.readBack === 'string') return o.readBack;
  const out: Record<string, unknown> = {};
  for (const [key] of o.lateAnswers) Object.assign(out, o.readBack[lateMap(key)] ?? {});
  return out;
}

/** The late keys answered with a refusal: the server turned them away before the WAL. */
function refusedLateKeys(o: Observed): string[] {
  return o.lateAnswers
    .filter(([, answer]) => answer === 'ERROR' || answer === 'OP_REJECTED')
    .map(([key]) => key);
}

/** What both arms assert about the first process and the healed read-back. */
function observed(o: Observed) {
  const lateRead = lateReadBack(o);
  const earlyRead = typeof o.readBack === 'string' ? o.readBack : o.readBack[EARLY_MAP];
  const refused = refusedLateKeys(o);
  return {
    exit: o.exit,
    exitedInsideTheDeadline:
      o.exitAfterReadOnlyMs !== null && o.exitAfterReadOnlyMs < FAILURE_DEADLINE_MS,
    storeBrokenLines: o.storeBrokenLines.length,
    storeBrokenLineNamesItsCause: o.storeBrokenLines.filter(
      (line) => line.includes('cause="io"') && line.includes('io_kind="Other"'),
    ).length,
    drainStoppedLines: o.drainStoppedLines.length,
    drainStoppedLineNamesItsCause: o.drainStoppedLines.filter((line) => /\bcause="\w+"/.test(line))
      .length,
    binaryBrokenLines: o.binaryBrokenLines.length,
    binaryBrokenLinesWithExitCode72: o.binaryBrokenLines.filter(
      (line) => /\bexit_code=72\b/.test(line) && line.includes('cause="io"'),
    ).length,
    panicLines: o.panicLines,
    // The listener closes as the node starts its stop, so a scrape after the
    // store's line may find nothing to answer; one that answers must count
    // the cause.
    brokenCounterWhenScraped: o.brokenCounter === null ? 'not answered' : o.brokenCounter,
    readinessNeverReadyAfterBroken: !o.readinessAfterBroken.includes('200'),
    // (i) at least one late key was acknowledged.
    someLateKeyAcked: Object.keys(lateAcked(o)).length > 0,
    // (iv) every early key is served, with its value.
    earlyReadBack: earlyRead,
    // (ii) the late map served is exactly the acknowledged late keys with
    // their values, which also says (iii): no refused late key is served.
    lateReadBack: lateRead,
    refusedLateKeysServed:
      typeof lateRead === 'string' ? lateRead : refused.filter((key) => key in lateRead),
    healedExitOnSigterm: o.healedExitOnSigterm,
    sha256BeforeEqualsAfter: o.sha256Before === o.sha256After,
  };
}

function expected(o: Observed) {
  return {
    exit: { code: 72, signal: null },
    exitedInsideTheDeadline: true,
    storeBrokenLines: 1,
    storeBrokenLineNamesItsCause: 1,
    drainStoppedLines: 1,
    drainStoppedLineNamesItsCause: 1,
    binaryBrokenLines: 1,
    binaryBrokenLinesWithExitCode72: 1,
    panicLines: 0,
    brokenCounterWhenScraped: o.brokenCounter === null ? 'not answered' : 1,
    readinessNeverReadyAfterBroken: true,
    someLateKeyAcked: true,
    earlyReadBack: o.acked[EARLY_MAP],
    lateReadBack: lateAcked(o),
    refusedLateKeysServed: [],
    healedExitOnSigterm: { code: 0, signal: null },
    sha256BeforeEqualsAfter: true,
  };
}

describe('Integration: a server whose RocksDB store is broken ends by itself with exit code 72', () => {
  test(
    'arm 1 LATCH-THEN-ORDERLY-EXIT',
    async () => {
      const o = await runArm('LATCH-THEN-ORDERLY-EXIT');
      if (o.preconditionNotMet !== null) {
        throw new ScenarioPreconditionError(o.arm, o.preconditionNotMet);
      }
      requireExitByItself(o);
      expect(observed(o)).toEqual(expected(o));
      expect(Object.keys(o.acked[EARLY_MAP] ?? {})).toEqual(EARLY_KEYS);
    },
    ARM_TIMEOUT_MS,
  );

  test(
    'arm 2 STILL-READ-ONLY-AT-RESTART',
    async () => {
      const o = await runArm('STILL-READ-ONLY-AT-RESTART');
      if (o.preconditionNotMet !== null) {
        throw new ScenarioPreconditionError(o.arm, o.preconditionNotMet);
      }
      requireExitByItself(o);
      expect({
        ...observed(o),
        restartOnTheReadOnlyDirectory: o.stillReadOnly,
      }).toEqual({
        ...expected(o),
        restartOnTheReadOnlyDirectory: {
          port: null,
          exit: { code: 1, signal: null },
          refusalAtStoreOpen: true,
          permissionNamed: true,
          diskFullNamed: false,
          walUnchanged: true,
        },
      });
      expect(Object.keys(o.acked[EARLY_MAP] ?? {})).toEqual(EARLY_KEYS);
    },
    ARM_TIMEOUT_MS,
  );
});
