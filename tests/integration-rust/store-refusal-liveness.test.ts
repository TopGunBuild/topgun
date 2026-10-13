/**
 * Integration: a write the store refuses stays with the server until the store
 * takes it.
 *
 * A write is acknowledged once it is in the write-ahead log; it reaches the
 * store later, in the background. This test makes the store refuse that later
 * step and then checks what an operator would care about: the server still
 * answers reads and new connections, the refused write is still readable, a
 * warning says that writes are being retried, the write reaches the store
 * once the store accepts writes again, and the server exits on SIGTERM.
 *
 * The refusal comes from a server built with the `fault-injection` cargo
 * feature: while a sentinel file exists, every store write is refused. No
 * release build has that feature, so the binary is given by path
 * (FAULT_SERVER_BINARY) together with the commit it was built from
 * (FAULT_SERVER_COMMIT). With either missing the test fails; it never skips
 * and never falls back to another binary.
 *
 * Six arms, one server process each except SIGTERM-WHILE-REFUSING, which has
 * two:
 *
 * - CONTROL  — the PINNED configuration with the sentinel never created, so
 *              nothing is refused. It shows the probes pass on a healthy node,
 *              that no retry warning is logged there, and that the retry gauge
 *              is published at zero before anything was ever refused.
 * - PINNED   — the stall watchdog ticks every 600 ms, so the lag gauge it
 *              refreshes follows the buffer closely;
 * - DEFAULTS — the shipped configuration (the watchdog ticks every 6 s);
 * - STORE-DOWN-FROM-START — the shipped configuration again, with the store
 *              refusing from the first flush on. See below.
 * - SIGTERM-WHILE-REFUSING — the PINNED configuration; the server is stopped
 *              while the store still refuses, and started again. See below.
 * - REFUSAL-LIFTED-UNDER-LOAD — the PINNED configuration; the store refuses
 *              for five seconds while six keys are rewritten four times a
 *              second, and the refusal is lifted while the writes continue.
 *              See below.
 *
 * PINNED and DEFAULTS store a control write while the store is healthy, then
 * have one write refused for fifteen seconds. While it is refused they read
 * two things before any probe connects: the retry warnings in the log, which
 * must exist and name the refused map only, and the two gauges
 * `topgun_write_behind_retrying_entries` and
 * `topgun_write_behind_buffered_entries`, which must both be above zero — the
 * write is still in the buffer. Then the refused key and the control key are
 * read back and a new connection is opened. Then the sentinel is removed, and
 * the per-partition gauge `topgun_wal_applied_watermark_lag` has to return to
 * zero on every partition, the log has to carry the line that says the retry
 * backlog cleared, and the retrying gauge has to read zero. A server that
 * dropped the refused write instead would show a lag that never returns to
 * zero without a restart, and a retrying gauge at zero while the store still
 * refuses.
 *
 * The readings taken while the store refuses come before the probes for a
 * reason: each connection makes the server write a device-credentials record
 * of its own, which is refused in its turn and may become the record a later
 * warning names.
 *
 * STORE-DOWN-FROM-START differs from those two on purpose. The sentinel is
 * created before the first connection and the refused write is sent as soon as
 * that connection is authenticated, so nothing was ever stored when the store
 * starts refusing. So:
 *
 * - there is no control write, and nothing is said about one (L0 is `n/a`);
 * - two records are refused, not one: the test's key, and the
 *   device-credentials record the server itself writes when the first
 *   connection authenticates. A retry warning names one record as its example
 *   and it may be either, so the warnings are checked by the maps they name:
 *   every one must name one of those two maps.
 *
 * SIGTERM-WHILE-REFUSING asks what a shutdown does with a write the store is
 * still refusing. The first process gets SIGTERM with the sentinel in place
 * and has to exit cleanly and in time. The sentinel is then removed and a
 * second process is started on the same data directory; the first thing done
 * against it is a read of the refused key, repeated until it returns the
 * written value. Nothing is asserted about the lag gauge of the second
 * process: a partition is given a lag sample only by a write made while the
 * process runs, so the gauge says nothing about a write recovered at start.
 *
 * REFUSAL-LIFTED-UNDER-LOAD asks a different question: once the store accepts
 * writes again, does the server finish accounting for every write it
 * acknowledged during the outage? A refused write stays buffered, and a newer
 * write of its key replaces the queued one; what the replaced write still
 * owed the write-ahead log has to pass to the newer one. The observable is the
 * per-partition gauge `topgun_wal_applied_watermark_lag` on `/metrics`: it is
 * zero before the outage, above zero during it, and has to be zero again on
 * every partition soon after the load stops. A partition that stays above zero
 * keeps its log from being reclaimed until the next restart. Each key must
 * read back its last written value. The retry warnings and the gauges are
 * recorded in this arm and not asserted: whether a warning falls inside five
 * seconds is not something the arm is about.
 *
 * Environment:
 *
 * - LIVENESS_ARMS    — comma-separated arms to run (default: all six);
 * - LIVENESS_OUT_DIR — where each run writes its notes and both server
 *                      streams; unset, nothing is written;
 * - LIVENESS_RUN     — label of the run in those file names;
 * - LIVENESS_SAMPLE  — `1` takes a stack sample of the server before SIGTERM
 *                      (macOS `/usr/bin/sample`; needs LIVENESS_OUT_DIR).
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  SEAM_BOOT_TEXT,
  LagScrape,
  allZero,
  lwwPut,
  requireFaultBinary,
  scrapeGauge,
  scrapeLag,
} from './helpers/fault-binary';
import { TestClient } from './helpers/test-client';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  observe,
  sleep,
  plainLines,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

type Arm =
  | 'PINNED'
  | 'DEFAULTS'
  | 'CONTROL'
  | 'STORE-DOWN-FROM-START'
  | 'SIGTERM-WHILE-REFUSING'
  | 'REFUSAL-LIFTED-UNDER-LOAD';
/** The arms that run one server process from start to SIGTERM. */
type SingleProcessArm = Exclude<Arm, 'SIGTERM-WHILE-REFUSING'>;
const ALL_ARMS: Arm[] = [
  'CONTROL',
  'PINNED',
  'DEFAULTS',
  'STORE-DOWN-FROM-START',
  'SIGTERM-WHILE-REFUSING',
  'REFUSAL-LIFTED-UNDER-LOAD',
];

const SAMPLE_TOOL = '/usr/bin/sample';

const CONTROL_MAP = 'liveness_control';
const CONTROL_KEY = 'alice';
const CONTROL_VALUE = { name: 'Alice', written: 'while the store was healthy' };
const REFUSED_MAP = 'liveness_refused';
const REFUSED_KEY = 'bob';
const REFUSED_VALUE = { name: 'Bob', written: 'while the store refuses' };
/** The map the server keeps its own per-device record in; it writes one when a connection authenticates. */
const DEVICE_CREDENTIALS_MAP = '_topgun_device_credentials';

const LOAD_MAP = 'liveness_load';
const LOAD_KEYS = ['load-0', 'load-1', 'load-2', 'load-3', 'load-4', 'load-5'];

/** Logged at warn level while the store refuses buffered writes; it names one refused record as an example. */
const RETRY_WARNING_TEXT = 'Write-behind entries are being retried: the store refuses them';
/** Logged once when the store has accepted every write it had refused, if a retry warning was logged before. */
const BACKLOG_CLEARED_TEXT =
  'Write-behind retry backlog cleared: the store accepted every refused entry';
/** Buffered writes the store has refused at least once, as of the last flush pass. */
const RETRYING_GAUGE = 'topgun_write_behind_retrying_entries';
/** Buffered keys, the number the capacity check uses. */
const BUFFERED_GAUGE = 'topgun_write_behind_buffered_entries';

/** Stands in an expectation for a key the arm has not written, so the member is visibly not a read. */
const NOT_READ = 'n/a: this arm does not write the key';

const ACK_WAIT_MS = 5_000;
const PROBE_WAIT_MS = 5_000;
/** Write delay (1 s) plus one flush tick (at most 1 s) plus slack: the control write is in the store by then. */
const CONTROL_FLUSH_WAIT_MS = 3_000;
/**
 * How long the store refuses the one write before anything is read. The retry
 * warning is due at the third refusal of one record, a few seconds after the
 * ack; the window leaves room, and it is long enough that a write merely
 * waiting for its first flush would be gone from the buffer.
 */
const WINDOW_MS = 15_000;
const SIGTERM_EXIT_MS = 15_000;
const SIGTERM_OBSERVE_MS = 10_000;
const CLIENT_CLOSE_MS = 2_000;
const TEST_TIMEOUT_MS = 120_000;

/** How long a baseline scrape with every lag at zero is waited for; the gauge is refreshed every 600 ms in this arm. */
const BASELINE_WAIT_MS = 3_000;
const BASELINE_POLL_MS = 200;
/**
 * The store refuses for this long under load. The first flush pass of the
 * outage comes one to two seconds in, so by the end refused writes are
 * waiting in the buffer while their keys are still being rewritten.
 */
const OUTAGE_MS = 5_000;
const LOAD_AFTER_LIFT_MS = 2_000;
const LOAD_ROUND_MS = 250;
/**
 * How long every lag is given to return to zero where the watchdog ticks every
 * 600 ms. Once the store accepts writes the buffer drains within six seconds,
 * and the gauge shows it one tick later.
 */
const SETTLE_WAIT_MS = 15_000;
/** The same wait where the watchdog ticks every 6 s: six seconds to drain, two ticks, and slack. */
const SETTLE_WAIT_DEFAULT_TICK_MS = 30_000;
const SETTLE_POLL_MS = 500;

function selectedArms(): Arm[] {
  const raw = process.env.LIVENESS_ARMS;
  if (!raw) return ALL_ARMS;
  const names = raw
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean);
  for (const name of names) {
    if (!ALL_ARMS.includes(name as Arm)) {
      throw new Error(
        `LIVENESS_ARMS names an unknown arm ${JSON.stringify(name)}; arms: ${ALL_ARMS.join(', ')}`,
      );
    }
  }
  if (names.length === 0) throw new Error('LIVENESS_ARMS is set and names no arm');
  return names as Arm[];
}

interface FaultEnv {
  binary: string;
  commit: string;
  outDir: string | null;
  sample: boolean;
  run: string;
}

/** Reads the run's configuration, and throws — never skips — when the seam binary is not given. */
function requireFaultEnv(): FaultEnv {
  const { binary, commit } = requireFaultBinary();

  const outDir = process.env.LIVENESS_OUT_DIR || null;
  const sample = process.env.LIVENESS_SAMPLE === '1';
  if (sample) {
    if (!outDir) {
      throw new Error(
        'LIVENESS_SAMPLE=1 needs LIVENESS_OUT_DIR: the stack sample has nowhere to go.',
      );
    }
    try {
      fs.accessSync(SAMPLE_TOOL, fs.constants.X_OK);
    } catch (err) {
      throw new Error(
        `LIVENESS_SAMPLE=1 needs ${SAMPLE_TOOL}, which is not executable here (${(err as Error).message}).`,
      );
    }
  }
  if (outDir) fs.mkdirSync(outDir, { recursive: true });
  const run =
    process.env.LIVENESS_RUN ||
    new Date()
      .toISOString()
      .replace(/[^0-9]/g, '')
      .slice(0, 14);
  return { binary, commit, outDir, sample, run };
}

/** Sends one write on an open connection and returns the type of the frame that answered it, or `null`. */
async function writeOn(
  client: TestClient,
  payload: Record<string, unknown>,
): Promise<string | null> {
  client.messages.length = 0;
  client.send({ type: 'CLIENT_OP', payload });
  const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
  return frame ? (frame.type as string) : null;
}

/** Closes a connection and waits for the socket to be gone; a server that no longer answers gets the socket destroyed. */
async function closeAndWait(client: TestClient): Promise<string> {
  const ws = client.ws;
  if (ws.readyState === ws.CLOSED) return 'already closed';
  const closed = new Promise<string>((resolve) => ws.once('close', () => resolve('closed')));
  client.close();
  const outcome = await Promise.race([closed, sleep(CLIENT_CLOSE_MS).then(() => 'destroyed')]);
  if (outcome === 'destroyed') ws.terminate();
  return outcome;
}

function takeSample(pid: number, file: string): Promise<string> {
  return new Promise((resolve) => {
    child_process.execFile(
      SAMPLE_TOOL,
      [String(pid), '2', '-file', file],
      { timeout: 30_000 },
      (err) => resolve(err ? `failed: ${err.message}` : 'taken'),
    );
  });
}

function isEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The notes of one run and its one-line record, written out whether the run ends or throws. */
interface RunRecord {
  notes: string[];
  summary: string;
  note(text: string): void;
}

function newRunRecord(arm: Arm, env: FaultEnv): RunRecord {
  const startedAt = Date.now();
  const record: RunRecord = {
    notes: [],
    summary: `store-refusal-liveness arm=${arm} commit=${env.commit} aborted=before the server was launched`,
    note(text: string) {
      record.notes.push(`${new Date().toISOString()} +${Date.now() - startedAt}ms ${text}`);
    },
  };
  return record;
}

function storeEnv(dataDir: string, sentinel: string, pinnedTick: boolean): Record<string, string> {
  const serverEnv: Record<string, string> = {
    STORAGE_BACKEND: 'redb',
    TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_FAULT_STORE_REFUSE_FILE: sentinel,
  };
  // The watchdog ticks at a tenth of the stall bound: 600 ms here, 6 s with the shipped bound.
  if (pinnedTick) serverEnv.TOPGUN_WAL_WATERMARK_STALL_BOUND_MS = '6000';
  return serverEnv;
}

interface Launch {
  server: SpawnedBinary;
  port: number;
  /** Taken just before the spawn call, so it includes the helper hashing the binary file. */
  spawnCallAt: number;
  /** Records why the run cannot go on and throws, with the server's stderr attached. */
  abort(reason: string): never;
}

/**
 * Boots one server and makes sure it is a binary that has the seam at all.
 * The process is handed to `track` as soon as it exists, so the caller can
 * end it even when this function throws.
 */
async function launch(
  arm: Arm,
  env: FaultEnv,
  label: string,
  serverEnv: Record<string, string>,
  record: RunRecord,
  track: (server: SpawnedBinary) => void,
): Promise<Launch> {
  const spawnCallAt = Date.now();
  const server = await spawnBinary({
    binaryPath: env.binary,
    commit: env.commit,
    label,
    env: serverEnv,
  });
  track(server);
  const abort = (reason: string): never => {
    record.summary = `store-refusal-liveness arm=${arm} sha256=${server.sha256} commit=${env.commit} aborted=${reason}`;
    throw new Error(`${reason}\n--- stderr ---\n${server.stderr()}`);
  };
  record.summary = `store-refusal-liveness arm=${arm} sha256=${server.sha256} commit=${env.commit} aborted=the run did not reach its probes`;
  if (server.port === null) abort(`the server (${label}) did not print PORT=`);
  const port = server.port as number;
  record.note(`${label}: PORT=${port} pid=${server.process.pid} sha256=${server.sha256}`);

  const seamDeadline = Date.now() + 2_000;
  const seamLine = () => plainLines(server.stdout()).find((line) => line.includes(SEAM_BOOT_TEXT));
  while (!seamLine() && Date.now() < seamDeadline) await sleep(25);
  if (!seamLine()) {
    abort(
      `the launched binary did not print its "${SEAM_BOOT_TEXT}" line: it has no store-refusal seam, so nothing it does is about a refused write`,
    );
  }
  record.note(`seam boot line: ${(seamLine() as string).slice(0, 300)}`);
  return { server, port, spawnCallAt, abort };
}

/** A write that reaches the store while it is healthy. */
async function storeControlWrite(
  writer: TestClient,
  launched: Launch,
  record: RunRecord,
): Promise<void> {
  const controlAck = await writeOn(writer, lwwPut('1', CONTROL_MAP, CONTROL_KEY, CONTROL_VALUE));
  record.note(`control write answered with ${controlAck}`);
  if (controlAck !== 'OP_ACK') {
    launched.abort(`the control write was answered with ${controlAck}, not OP_ACK`);
  }
  await sleep(CONTROL_FLUSH_WAIT_MS);
}

/**
 * The write the store will refuse. It is acknowledged all the same: the WAL
 * has it. Returns when it was acknowledged and how long after the spawn call
 * that was.
 */
async function sendRefusedWrite(
  writer: TestClient,
  launched: Launch,
  record: RunRecord,
): Promise<{ ackedAt: number; delaySpawnMs: number }> {
  const refusedAck = await writeOn(writer, lwwPut('2', REFUSED_MAP, REFUSED_KEY, REFUSED_VALUE));
  const ackedAt = Date.now();
  const delaySpawnMs = ackedAt - launched.spawnCallAt;
  record.note(
    `refused-map write answered with ${refusedAck}, ${delaySpawnMs} ms after the spawn call`,
  );
  if (refusedAck !== 'OP_ACK') {
    launched.abort(
      `the write to ${REFUSED_MAP} was answered with ${refusedAck}, not OP_ACK within ${ACK_WAIT_MS} ms: the seam disturbs the acknowledgement path`,
    );
  }
  return { ackedAt, delaySpawnMs };
}

/** What the log and the two gauges say about refused writes at one moment. */
interface RefusalReading {
  /** Number of retry warnings in the log so far. */
  retryWarnings: number;
  /** The maps those warnings name as their example, each once, sorted. */
  warningMaps: string[];
  /** `null` when the server does not render the series. */
  retryingEntries: number | null;
  bufferedEntries: number | null;
}

function retryWarningLines(server: SpawnedBinary): string[] {
  return plainLines(server.stdout()).filter((line) => line.includes(RETRY_WARNING_TEXT));
}

async function readRefusal(server: SpawnedBinary, port: number): Promise<RefusalReading> {
  const warnings = retryWarningLines(server);
  const maps = new Set<string>();
  for (const line of warnings) {
    const named = / map=(\S+) key=/.exec(line);
    maps.add(named ? named[1] : '(unparsed)');
  }
  return {
    retryWarnings: warnings.length,
    warningMaps: [...maps].sort(),
    retryingEntries: await scrapeGauge(port, RETRYING_GAUGE),
    bufferedEntries: await scrapeGauge(port, BUFFERED_GAUGE),
  };
}

function shownReading(reading: RefusalReading): string {
  return (
    `retry_warnings=${reading.retryWarnings} warning_maps=${JSON.stringify(reading.warningMaps)} ` +
    `retrying=${reading.retryingEntries} buffered=${reading.bufferedEntries}`
  );
}

/**
 * Scrapes the lag gauge until one scrape has every partition at zero, for at
 * most `boundMs` counted from `from`. A sequence nothing accounts for never
 * resolves, so its partition never returns to zero: waiting longer cannot turn
 * a leak into a pass.
 */
async function waitForLagsAtZero(
  port: number,
  from: number,
  boundMs: number,
): Promise<{ settledAfterMs: number | null; scrapes: LagScrape[] }> {
  const scrapes: LagScrape[] = [];
  let settledAfterMs: number | null = null;
  while (settledAfterMs === null && Date.now() < from + boundMs) {
    await sleep(SETTLE_POLL_MS);
    const scrape = await scrapeLag(port);
    scrapes.push(scrape);
    if (allZero(scrape)) settledAfterMs = Date.now() - from;
  }
  return { settledAfterMs, scrapes };
}

/** What an arm saw after the store began accepting writes again. */
interface AfterLift {
  settleBoundMs: number;
  allLagsZeroWithinBound: boolean;
  /** Time from the start of the wait to the first scrape with every lag at zero, or `null`. */
  settledAfterMs: number | null;
  lastScrapes: LagScrape[];
  /** Number of lines in the log that say the retry backlog cleared. */
  backlogClearedLines: number;
  retryingEntries: number | null;
}

async function readAfterLift(
  server: SpawnedBinary,
  port: number,
  settleBoundMs: number,
  settled: { settledAfterMs: number | null; scrapes: LagScrape[] },
): Promise<AfterLift> {
  return {
    settleBoundMs,
    allLagsZeroWithinBound: settled.settledAfterMs !== null,
    settledAfterMs: settled.settledAfterMs,
    lastScrapes: settled.scrapes.slice(-5),
    backlogClearedLines: plainLines(server.stdout()).filter((line) =>
      line.includes(BACKLOG_CLEARED_TEXT),
    ).length,
    retryingEntries: await scrapeGauge(port, RETRYING_GAUGE),
  };
}

/**
 * The test's own connections go before a SIGTERM, so the shutdown is not
 * measured while it waits for them. A read probe closes its own connection
 * when it returns; the pause lets that close land as well.
 */
async function closeClients(clients: TestClient[], record: RunRecord): Promise<void> {
  for (const client of clients.splice(0)) {
    record.note(`client ${client.nodeId}: ${await closeAndWait(client)}`);
  }
  await sleep(500);
}

interface Stopped {
  runningBeforeSigterm: boolean;
  s0: number;
  s10: number;
  x10: 'running' | 'exited';
  exit: BinaryExit | null;
  /** Time from sending SIGTERM to the process being gone (after a SIGKILL, if it took one). */
  exitMs: number;
  sampleName: string;
}

/**
 * Optionally records where every thread of the server is, then sends SIGTERM
 * and watches whether the server writes anything or exits in the ten seconds
 * after it.
 */
async function sampleAndStop(
  launched: Launch,
  env: FaultEnv,
  sampleStem: string,
  record: RunRecord,
): Promise<Stopped> {
  const server = launched.server;
  let sampleName = 'off';
  if (env.sample) {
    sampleName = `${sampleStem}.sample.txt`;
    const taken = await takeSample(
      server.process.pid as number,
      path.join(env.outDir as string, sampleName),
    );
    record.note(`stack sample ${sampleName}: ${taken}`);
    if (taken !== 'taken') {
      launched.abort(`LIVENESS_SAMPLE=1 and the stack sample was not taken (${taken})`);
    }
  }

  const runningBeforeSigterm = server.exit() === null;
  const s0 = Buffer.byteLength(server.stdout());
  record.note(`SIGTERM (running=${runningBeforeSigterm}, stdout ${s0} bytes)`);
  const signalledAt = Date.now();
  const stopping = server.stop('SIGTERM', SIGTERM_EXIT_MS).then((status) => ({
    status,
    exitMs: Date.now() - signalledAt,
  }));
  await sleep(SIGTERM_OBSERVE_MS);
  const s10 = Buffer.byteLength(server.stdout());
  const x10 = server.exit() === null ? 'running' : 'exited';
  const { status: exit, exitMs } = await stopping;
  record.note(
    `after SIGTERM: stdout ${s10} bytes at +10 s, ${x10}; exit ${JSON.stringify(exit)} ${exitMs} ms after the signal`,
  );
  return { runningBeforeSigterm, s0, s10, x10, exit, exitMs, sampleName };
}

/** What the arm that lifts the refusal under load saw, beyond the probes every arm has. */
interface LoadObserved {
  lagSamplesAtBaseline: number;
  lagAtBaseline: LagScrape;
  /** The largest lag of any partition just before the refusal was lifted. */
  maxLagDuringOutage: number;
  rounds: number;
  allLagsZeroWithinBound: boolean;
  /** Time from the last load write to the first scrape with every lag at zero, or `null`. */
  settledAfterMs: number | null;
  lastScrapes: LagScrape[];
  /** Partitions whose lag was 2 or more in each of the last five scrapes. */
  stuckPartitions: string[];
  /** For each load key, the last value written and the value read back afterwards. */
  written: Record<string, unknown>;
  readBack: Record<string, unknown>;
}

interface Observed {
  arm: SingleProcessArm;
  /** `ok` when no retry warning names the control map; `n/a` in the arm that has no control write. */
  l0: 'ok' | 'fail' | 'n/a';
  /** The control key as read back, the reason it could not be read, or `NOT_READ`. */
  controlRead: unknown;
  /** The refused key as read back while the store still refuses it, the reason it could not be read, or `NOT_READ`. */
  refusedRead: unknown;
  /** `ok` when a new connection completed its handshake, otherwise the reason. */
  l2: string;
  /**
   * The log and the gauges before any probe connected: while the store still
   * refuses in the arms that refuse, at the end of the window in CONTROL.
   */
  before: RefusalReading;
  /** Set in the arms that remove the sentinel, `null` in CONTROL. */
  afterLift: AfterLift | null;
  runningBeforeSigterm: boolean;
  s0: number;
  s10: number;
  x10: 'running' | 'exited';
  l4: BinaryExit | null;
  /** Time from sending SIGTERM to the process being gone (after a SIGKILL, if it took one). */
  exitMs: number;
  verdict: 'HUNG' | 'HEALTHY' | 'OTHER';
  /** Set in the arm that lifts the refusal under load, `null` in every other. */
  load: LoadObserved | null;
}

function writeRunFiles(
  env: FaultEnv,
  stem: string,
  record: RunRecord,
  servers: SpawnedBinary[],
): void {
  if (!env.outDir) return;
  const base = path.join(env.outDir, stem);
  fs.writeFileSync(`${base}.notes.txt`, `${record.notes.join('\n')}\n${record.summary}\n`);
  // The first process keeps the plain names; a second one of the same arm is the restart.
  const names = ['', '.restart'];
  for (let i = 0; i < Math.max(1, servers.length); i += 1) {
    const server: SpawnedBinary | undefined = servers[i];
    fs.writeFileSync(`${base}${names[i] ?? `.${i}`}.stdout.txt`, server ? server.stdout() : '');
    fs.writeFileSync(`${base}${names[i] ?? `.${i}`}.stderr.txt`, server ? server.stderr() : '');
  }
}

async function runArm(arm: SingleProcessArm): Promise<Observed> {
  const env = requireFaultEnv();
  const fromStart = arm === 'STORE-DOWN-FROM-START';
  const liftedUnderLoad = arm === 'REFUSAL-LIFTED-UNDER-LOAD';
  const pinnedTick = arm === 'PINNED' || arm === 'CONTROL' || liftedUnderLoad;
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-store-refusal-'));
  const sentinel = path.join(dataDir, 'refuse');
  const record = newRunRecord(arm, env);
  const note = record.note;
  const clients: TestClient[] = [];
  const servers: SpawnedBinary[] = [];

  try {
    // Step 1: boot. The clock for "how early was the refused write" starts
    // just before the spawn call.
    const launched = await launch(
      arm,
      env,
      `store-refusal ${arm}`,
      storeEnv(dataDir, sentinel, pinnedTick),
      record,
      (server) => servers.push(server),
    );
    const { server, port, abort } = launched;

    let writer: TestClient;
    if (fromStart) {
      // Armed before any write exists, so every flush of this run is refused,
      // the server's own first write included. No control write: it would need
      // a healthy flush first.
      fs.writeFileSync(sentinel, '');
      note('sentinel created before the first connection: store writes are refused from the start');
      writer = await connectWithin(port, 'liveness-writer', PROBE_WAIT_MS);
      clients.push(writer);
    } else {
      // Step 2: a write that reaches the store while it is healthy.
      writer = await connectWithin(port, 'liveness-writer', PROBE_WAIT_MS);
      clients.push(writer);
      await storeControlWrite(writer, launched, record);

      // Step 3: from here on the store refuses every write. The arm that
      // lifts the refusal under load arms it further down, after its baseline
      // scrape.
      if (arm === 'CONTROL') {
        note('CONTROL: sentinel not created');
      } else if (!liftedUnderLoad) {
        fs.writeFileSync(sentinel, '');
        note('sentinel created: store writes are refused');
      }
    }

    let delaySpawnMs: number | null = null;
    let load: LoadObserved | null = null;
    let before: RefusalReading | null = null;
    let afterLift: AfterLift | null = null;
    if (liftedUnderLoad) {
      // A clean start: the gauge is being published and no partition lags. An
      // empty scrape must not pass for "all zero".
      let baseline: LagScrape = {};
      const baselineDeadline = Date.now() + BASELINE_WAIT_MS;
      for (;;) {
        baseline = await scrapeLag(port);
        if (allZero(baseline) || Date.now() >= baselineDeadline) break;
        await sleep(BASELINE_POLL_MS);
      }
      note(`baseline lag ${JSON.stringify(baseline)}`);
      if (!allZero(baseline)) {
        abort(
          `no scrape within ${BASELINE_WAIT_MS} ms had a lag sample with every sample at 0 (last: ${JSON.stringify(baseline)}): the run does not start clean, or the gauge is not published`,
        );
      }

      fs.writeFileSync(sentinel, '');
      const t0 = Date.now();
      note('sentinel created: store writes are refused');

      // Every key is rewritten once a round on the one open connection; a new
      // connection would make the server write a record of its own.
      const written: Record<string, unknown> = {};
      let maxLagDuringOutage = 0;
      let liftedAt: number | null = null;
      let round = 0;
      for (;;) {
        if (liftedAt === null && Date.now() >= t0 + OUTAGE_MS) {
          const outage = await scrapeLag(port);
          maxLagDuringOutage = Math.max(0, ...Object.values(outage));
          before = await readRefusal(server, port);
          fs.rmSync(sentinel);
          liftedAt = Date.now();
          note(
            `sentinel removed after round ${round}: store writes are accepted again; lag just before ${JSON.stringify(outage)}; ${shownReading(before)}`,
          );
        }
        if (liftedAt !== null && Date.now() >= liftedAt + LOAD_AFTER_LIFT_MS) break;
        for (const key of LOAD_KEYS) {
          const value = { round };
          const ack = await writeOn(writer, lwwPut(`load-${round}-${key}`, LOAD_MAP, key, value));
          if (ack !== 'OP_ACK') {
            abort(`round ${round}: the write to ${key} was answered with ${ack}, not OP_ACK`);
          }
          written[key] = value;
        }
        round += 1;
        await sleep(Math.max(0, t0 + round * LOAD_ROUND_MS - Date.now()));
      }
      const tEnd = Date.now();
      note(`load stopped after ${round} rounds`);

      const settled = await waitForLagsAtZero(port, tEnd, SETTLE_WAIT_MS);
      afterLift = await readAfterLift(server, port, SETTLE_WAIT_MS, settled);
      const lastScrapes = afterLift.lastScrapes;
      const stuckPartitions = Object.keys(lastScrapes[0] ?? {})
        .filter((partition) => lastScrapes.every((scrape) => scrape[partition] >= 2))
        .sort((a, b) => Number(a) - Number(b));
      note(
        `lag settled after ${settled.settledAfterMs} ms; ${settled.scrapes.length} scrapes, the last ${lastScrapes.length}: ${JSON.stringify(lastScrapes)}`,
      );
      note(`partitions at lag 2 or more in each of those: ${JSON.stringify(stuckPartitions)}`);
      note(
        `after the lift: backlog-cleared lines ${afterLift.backlogClearedLines}, retrying ${afterLift.retryingEntries}`,
      );
      load = {
        lagSamplesAtBaseline: Object.keys(baseline).length,
        lagAtBaseline: baseline,
        maxLagDuringOutage,
        rounds: round,
        allLagsZeroWithinBound: settled.settledAfterMs !== null,
        settledAfterMs: settled.settledAfterMs,
        lastScrapes,
        stuckPartitions,
        written,
        readBack: {},
      };
    } else {
      // Step 4: the write the store will refuse.
      const { ackedAt, delaySpawnMs: delay } = await sendRefusedWrite(writer, launched, record);
      delaySpawnMs = delay;

      // Step 5: the store keeps refusing it for the whole window.
      await sleep(Math.max(0, ackedAt + WINDOW_MS - Date.now()));
      note('end of the window');
    }

    // Step 6: what the log and the gauges say, taken before the probes
    // connect: each probe connection makes the server write another
    // device-credentials record, which is refused in its turn and can become
    // the record a warning names.
    if (before === null) {
      before = await readRefusal(server, port);
      note(`before the probes: ${shownReading(before)}`);
    }
    const controlWarnings = retryWarningLines(server).filter((line) =>
      line.includes(` map=${CONTROL_MAP} `),
    );
    let l0: Observed['l0'] = controlWarnings.length === 0 ? 'ok' : 'fail';
    if (fromStart) l0 = 'n/a';

    // Step 7: probes, collected first and asserted together by the caller.
    // The refused key is read while the store still refuses it.
    let refusedRead: unknown = NOT_READ;
    if (!liftedUnderLoad) {
      refusedRead = await observe(() =>
        queryKey(port, REFUSED_MAP, REFUSED_KEY, 'liveness-read-refused', PROBE_WAIT_MS),
      );
    }
    let controlRead: unknown = NOT_READ;
    if (!fromStart) {
      controlRead = await observe(() =>
        queryKey(port, CONTROL_MAP, CONTROL_KEY, 'liveness-read', PROBE_WAIT_MS),
      );
    }
    note(`L1 control ${JSON.stringify(controlRead)} refused ${JSON.stringify(refusedRead)}`);
    const l2Client = await observe(() => connectWithin(port, 'liveness-connect', PROBE_WAIT_MS));
    let l2: string;
    if (typeof l2Client === 'string') {
      l2 = l2Client;
    } else {
      clients.push(l2Client);
      l2 = 'ok';
    }
    note(`L2 ${l2}`);
    note(`L0 ${l0}`);
    if (load) {
      for (const key of LOAD_KEYS) {
        load.readBack[key] = await observe(() =>
          queryKey(port, LOAD_MAP, key, 'liveness-read', PROBE_WAIT_MS),
        );
      }
      note(
        `load keys written ${JSON.stringify(load.written)} read ${JSON.stringify(load.readBack)}`,
      );
    }

    // Step 8: the store accepts writes again. Everything it refused — the
    // records the probes made the server write included — has to reach it.
    if (arm === 'PINNED' || arm === 'DEFAULTS' || fromStart) {
      const settleBoundMs = arm === 'PINNED' ? SETTLE_WAIT_MS : SETTLE_WAIT_DEFAULT_TICK_MS;
      fs.rmSync(sentinel);
      const liftedAt = Date.now();
      note('sentinel removed: store writes are accepted again');
      const settled = await waitForLagsAtZero(port, liftedAt, settleBoundMs);
      afterLift = await readAfterLift(server, port, settleBoundMs, settled);
      note(
        `lag settled ${settled.settledAfterMs} ms after the lift (bound ${settleBoundMs} ms); ${settled.scrapes.length} scrapes, the last ${afterLift.lastScrapes.length}: ${JSON.stringify(afterLift.lastScrapes)}`,
      );
      note(
        `after the lift: backlog-cleared lines ${afterLift.backlogClearedLines}, retrying ${afterLift.retryingEntries}`,
      );
    }

    // Step 9: the test's own connections go first, then SIGTERM.
    await closeClients(clients, record);
    const stopped = await sampleAndStop(launched, env, `${arm}-${env.run}`, record);
    const l4 = stopped.exit;

    const l1Ok =
      (fromStart || isEqual(controlRead, CONTROL_VALUE)) &&
      (liftedUnderLoad || isEqual(refusedRead, REFUSED_VALUE));
    const l4Clean = isEqual(l4, { code: 0, signal: null });
    let verdict: Observed['verdict'] = 'OTHER';
    if (!l1Ok && l2 !== 'ok' && l4?.signal === 'SIGKILL') verdict = 'HUNG';
    else if (l0 !== 'fail' && l1Ok && l2 === 'ok' && l4Clean) verdict = 'HEALTHY';

    record.summary =
      `store-refusal-liveness arm=${arm} sha256=${server.sha256} commit=${env.commit} ` +
      `delay_spawn_ms=${delaySpawnMs ?? 'n/a'} ` +
      `L0=${l0} L1=${l1Ok ? 'ok' : 'fail'} L2=${l2 === 'ok' ? 'ok' : 'fail'} ` +
      `${shownReading(before)} ` +
      `S0=${stopped.s0} S10=${stopped.s10} X10=${stopped.x10} L4=${JSON.stringify(l4)} exit_ms=${stopped.exitMs} ` +
      `sample=${stopped.sampleName} verdict=${verdict}`;
    if (afterLift) {
      record.summary +=
        ` settleBoundMs=${afterLift.settleBoundMs} settledAfterMs=${afterLift.settledAfterMs} ` +
        `backlogClearedLines=${afterLift.backlogClearedLines} retryingAfterLift=${afterLift.retryingEntries}`;
    }
    if (load) {
      record.summary +=
        ` lagSamplesAtBaseline=${load.lagSamplesAtBaseline} maxLagDuringOutage=${load.maxLagDuringOutage} ` +
        `rounds=${load.rounds} allLagsZeroWithinBound=${load.allLagsZeroWithinBound} ` +
        `stuckPartitions=${JSON.stringify(load.stuckPartitions)}`;
    }

    return {
      arm,
      l0,
      controlRead,
      refusedRead,
      l2,
      before,
      afterLift,
      runningBeforeSigterm: stopped.runningBeforeSigterm,
      s0: stopped.s0,
      s10: stopped.s10,
      x10: stopped.x10,
      l4,
      exitMs: stopped.exitMs,
      verdict,
      load,
    };
  } finally {
    for (const client of clients) client.close();
    for (const server of servers) await server.stop('SIGKILL', 5_000);
    // Straight to stderr: the suite silences console.log, and this line is the record of the run.
    process.stderr.write(`${record.summary}\n`);
    writeRunFiles(env, `${arm}-${env.run}`, record, servers);
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

/** What the arm that stops the server while the store still refuses saw, in its two processes. */
interface RestartObserved {
  /** The log and the gauges of the first process just before the signal, with no probe connected. */
  before: RefusalReading;
  first: Stopped;
  /** The refused key as last read from the second process, or the reason it could not be read. */
  restartRead: unknown;
  /** Time from the second process being up to the read that returned the written value, or `null`. */
  restartReadAfterMs: number | null;
  restartReadAttempts: number;
  second: Stopped;
}

/**
 * The arm with two processes. It cannot go through `runArm`, which ends its
 * one process and deletes the data directory together: here the directory has
 * to outlive the first process.
 */
async function runSigtermWhileRefusing(): Promise<RestartObserved> {
  const arm: Arm = 'SIGTERM-WHILE-REFUSING';
  const env = requireFaultEnv();
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-store-refusal-'));
  const sentinel = path.join(dataDir, 'refuse');
  const record = newRunRecord(arm, env);
  const note = record.note;
  const clients: TestClient[] = [];
  const servers: SpawnedBinary[] = [];
  const serverEnv = storeEnv(dataDir, sentinel, true);

  try {
    const first = await launch(arm, env, `store-refusal ${arm}`, serverEnv, record, (server) =>
      servers.push(server),
    );
    const writer = await connectWithin(first.port, 'liveness-writer', PROBE_WAIT_MS);
    clients.push(writer);
    await storeControlWrite(writer, first, record);
    fs.writeFileSync(sentinel, '');
    note('sentinel created: store writes are refused');
    const { ackedAt, delaySpawnMs } = await sendRefusedWrite(writer, first, record);
    await sleep(Math.max(0, ackedAt + WINDOW_MS - Date.now()));
    note('end of the window');

    const before = await readRefusal(first.server, first.port);
    note(`before the signal: ${shownReading(before)}`);

    // The signal reaches a server whose store still refuses: the sentinel stays.
    await closeClients(clients, record);
    const firstStopped = await sampleAndStop(first, env, `${arm}-${env.run}`, record);

    fs.rmSync(sentinel);
    note('sentinel removed; starting a second process on the same data directory');
    const second = await launch(
      arm,
      env,
      `store-refusal ${arm} restart`,
      serverEnv,
      record,
      (server) => servers.push(server),
    );

    // The read comes before any other connection to the second process, so
    // what it returns is what the start recovered and nothing a later write
    // of this test put there.
    const readFrom = Date.now();
    let restartRead: unknown = 'NOT OBSERVED: no read was made';
    let restartReadAfterMs: number | null = null;
    let restartReadAttempts = 0;
    for (;;) {
      const left = readFrom + SETTLE_WAIT_MS - Date.now();
      if (left <= 0) break;
      restartReadAttempts += 1;
      restartRead = await observe(() =>
        queryKey(
          second.port,
          REFUSED_MAP,
          REFUSED_KEY,
          'liveness-read-restart',
          Math.min(PROBE_WAIT_MS, left),
        ),
      );
      if (isEqual(restartRead, REFUSED_VALUE)) {
        restartReadAfterMs = Date.now() - readFrom;
        break;
      }
      await sleep(SETTLE_POLL_MS);
    }
    note(
      `restart: refused key read ${JSON.stringify(restartRead)} after ${restartReadAfterMs} ms, ${restartReadAttempts} attempts`,
    );

    await closeClients(clients, record);
    const secondStopped = await sampleAndStop(second, env, `${arm}-restart-${env.run}`, record);

    record.summary =
      `store-refusal-liveness arm=${arm} sha256=${first.server.sha256} commit=${env.commit} ` +
      `delay_spawn_ms=${delaySpawnMs} ${shownReading(before)} ` +
      `S0=${firstStopped.s0} S10=${firstStopped.s10} X10=${firstStopped.x10} ` +
      `L4=${JSON.stringify(firstStopped.exit)} exit_ms=${firstStopped.exitMs} sample=${firstStopped.sampleName} ` +
      `restart_sha256=${second.server.sha256} ` +
      `restart_read=${isEqual(restartRead, REFUSED_VALUE) ? 'ok' : 'fail'} restart_read_ms=${restartReadAfterMs} ` +
      `restart_read_attempts=${restartReadAttempts} ` +
      `restart_L4=${JSON.stringify(secondStopped.exit)} restart_exit_ms=${secondStopped.exitMs} ` +
      `restart_sample=${secondStopped.sampleName}`;

    return {
      before,
      first: firstStopped,
      restartRead,
      restartReadAfterMs,
      restartReadAttempts,
      second: secondStopped,
    };
  } finally {
    for (const client of clients) client.close();
    for (const server of servers) await server.stop('SIGKILL', 5_000);
    // Straight to stderr: the suite silences console.log, and this line is the record of the run.
    process.stderr.write(`${record.summary}\n`);
    writeRunFiles(env, `${arm}-${env.run}`, record, servers);
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

describe('Integration: a write the store refuses stays buffered, is retried and reaches the store', () => {
  for (const arm of selectedArms()) {
    if (arm === 'SIGTERM-WHILE-REFUSING') {
      test(
        'SIGTERM-WHILE-REFUSING: a SIGTERM while the store still refuses ends the process cleanly, and the refused write is read back after a restart',
        async () => {
          const o = await runSigtermWhileRefusing();
          expect({
            retryWarningNamesTheRefusedMap: o.before.warningMaps.includes(REFUSED_MAP),
            retryingEntriesBeforeSigterm: (o.before.retryingEntries ?? 0) >= 1,
            runningBeforeSigterm: o.first.runningBeforeSigterm,
            exitOnSigterm: o.first.exit,
            exitedWithinBound: o.first.exitMs < SIGTERM_EXIT_MS,
            readAfterRestart: o.restartRead,
            readAfterRestartWithinBound: o.restartReadAfterMs !== null,
            restartExitOnSigterm: o.second.exit,
          }).toEqual({
            retryWarningNamesTheRefusedMap: true,
            retryingEntriesBeforeSigterm: true,
            runningBeforeSigterm: true,
            exitOnSigterm: { code: 0, signal: null },
            exitedWithinBound: true,
            readAfterRestart: REFUSED_VALUE,
            readAfterRestartWithinBound: true,
            restartExitOnSigterm: { code: 0, signal: null },
          });
        },
        TEST_TIMEOUT_MS,
      );
      continue;
    }

    if (arm === 'CONTROL') {
      test(
        'CONTROL: with nothing refused, every probe passes, no retry warning is logged and the retry gauge reads zero',
        async () => {
          const o = await runArm(arm);
          expect({
            controlWriteNamedInWarning: o.l0 === 'fail',
            retryWarnings: o.before.retryWarnings,
            // Zero, not absent: the series is published before anything was refused.
            retryingEntries: o.before.retryingEntries,
            readControl: o.controlRead,
            readRefused: o.refusedRead,
            newConnection: o.l2,
            runningBeforeSigterm: o.runningBeforeSigterm,
            exitOnSigterm: o.l4,
            exitedWithinBound: o.exitMs < SIGTERM_EXIT_MS,
            verdict: o.verdict,
          }).toEqual({
            controlWriteNamedInWarning: false,
            retryWarnings: 0,
            retryingEntries: 0,
            readControl: CONTROL_VALUE,
            readRefused: REFUSED_VALUE,
            newConnection: 'ok',
            runningBeforeSigterm: true,
            exitOnSigterm: { code: 0, signal: null },
            exitedWithinBound: true,
            verdict: 'HEALTHY',
          });
        },
        TEST_TIMEOUT_MS,
      );
      continue;
    }

    if (arm === 'REFUSAL-LIFTED-UNDER-LOAD') {
      test(
        'REFUSAL-LIFTED-UNDER-LOAD: once the store accepts writes again, every partition catches up with its log and each key reads back its last written value',
        async () => {
          const o = await runArm(arm);
          const load = o.load as LoadObserved;
          expect({
            lagSamplesAtBaseline: load.lagSamplesAtBaseline >= 1,
            lagAtBaselineAllZero: allZero(load.lagAtBaseline),
            lagDuringOutage: load.maxLagDuringOutage >= 1,
            allLagsZeroWithinBound: load.allLagsZeroWithinBound,
            controlWriteNamedInWarning: o.l0 === 'fail',
            read: o.controlRead,
            newConnection: o.l2,
            loadKeys: load.readBack,
            runningBeforeSigterm: o.runningBeforeSigterm,
            exitOnSigterm: o.l4,
            exitedWithinBound: o.exitMs < SIGTERM_EXIT_MS,
            verdict: o.verdict,
          }).toEqual({
            lagSamplesAtBaseline: true,
            lagAtBaselineAllZero: true,
            lagDuringOutage: true,
            allLagsZeroWithinBound: true,
            controlWriteNamedInWarning: false,
            read: CONTROL_VALUE,
            newConnection: 'ok',
            loadKeys: load.written,
            runningBeforeSigterm: true,
            exitOnSigterm: { code: 0, signal: null },
            exitedWithinBound: true,
            verdict: 'HEALTHY',
          });
        },
        TEST_TIMEOUT_MS,
      );
      continue;
    }

    // With the store down from the start the server's own device-credentials
    // record is refused alongside the test's key, and a warning names either.
    const refusedMaps =
      arm === 'STORE-DOWN-FROM-START' ? [REFUSED_MAP, DEVICE_CREDENTIALS_MAP] : [REFUSED_MAP];
    let title = `${arm}: a refused write stays readable and buffered, is reported by the retry warning, and reaches the store once the refusal is lifted`;
    if (arm === 'STORE-DOWN-FROM-START') {
      title =
        'STORE-DOWN-FROM-START: the store refuses from the start, default watchdog tick — the server keeps serving, warns about a record it retries, and stores what it buffered once the refusal is lifted';
    }
    test(
      title,
      async () => {
        const o = await runArm(arm);
        const after = o.afterLift as AfterLift;
        expect({
          controlWriteNamedInWarning: o.l0 === 'fail',
          retryWarningBeforeLift: o.before.retryWarnings >= 1,
          warningMapsOtherThanTheRefusedOnes: o.before.warningMaps.filter(
            (map) => !refusedMaps.includes(map),
          ),
          retryingEntriesBeforeLift: (o.before.retryingEntries ?? 0) >= 1,
          bufferedEntriesBeforeLift: (o.before.bufferedEntries ?? 0) >= 1,
          readRefused: o.refusedRead,
          readControl: o.controlRead,
          newConnection: o.l2,
          allLagsZeroAfterLift: after.allLagsZeroWithinBound,
          backlogClearedLineAfterLift: after.backlogClearedLines >= 1,
          retryingEntriesAfterLift: after.retryingEntries,
          runningBeforeSigterm: o.runningBeforeSigterm,
          exitOnSigterm: o.l4,
          exitedWithinBound: o.exitMs < SIGTERM_EXIT_MS,
          verdict: o.verdict,
        }).toEqual({
          controlWriteNamedInWarning: false,
          retryWarningBeforeLift: true,
          warningMapsOtherThanTheRefusedOnes: [],
          retryingEntriesBeforeLift: true,
          bufferedEntriesBeforeLift: true,
          readRefused: REFUSED_VALUE,
          readControl: arm === 'STORE-DOWN-FROM-START' ? NOT_READ : CONTROL_VALUE,
          newConnection: 'ok',
          allLagsZeroAfterLift: true,
          backlogClearedLineAfterLift: true,
          retryingEntriesAfterLift: 0,
          runningBeforeSigterm: true,
          exitOnSigterm: { code: 0, signal: null },
          exitedWithinBound: true,
          verdict: 'HEALTHY',
        });
      },
      TEST_TIMEOUT_MS,
    );
  }
});
