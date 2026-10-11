/**
 * Integration: a server whose background flush task has panicked stops in
 * order and says so with its exit code.
 *
 * A write is acknowledged once it is in the write-ahead log; a background task
 * carries it to the store about a second later. When that task dies, nothing
 * would flush again while the process lives. This test makes it die and then
 * checks what a process supervisor and an operator would see: the process ends
 * by itself, with a code that is not a clean stop, and after a restart every
 * acknowledged write is readable.
 *
 * The panic comes from a server built with the `fault-injection` cargo
 * feature: while a sentinel file exists, persisting an entry panics. The
 * binary is given by path (FAULT_SERVER_BINARY) together with the commit it
 * was built from (FAULT_SERVER_COMMIT). With either missing the test fails; it
 * never skips and never falls back to another binary.
 *
 * Two arms, one server process each. The sentinel is removed in neither, so
 * what tells them apart is not the sentinel: the flush task is dead in both,
 * and the only code left that persists an entry is the drain at shutdown. That
 * drain is reached only when something is still queued, and a flush pass takes
 * every entry at least one second old. So the arms differ by what is queued
 * when the task dies:
 *
 * - PANIC-THEN-ORDERLY-EXIT — one acknowledged write after the sentinel and
 *   nothing else. The dying pass holds that one entry, the queue is empty, the
 *   shutdown has nothing to persist and completes: exit code 70.
 * - PANIC-IN-SHUTDOWN — a continuous stream of distinct keys. Entries younger
 *   than a second are still queued when the task dies, the shutdown drain meets
 *   the sentinel and panics as well: exit code 71.
 *
 * Each arm checks that its own setup held (its precondition) before it says
 * anything. A precondition that did not hold is an error of the scenario — the
 * run is neither a pass nor a failure of the arm — and is thrown as
 * `ScenarioPreconditionError`, whose message starts with
 * `SCENARIO PRECONDITION NOT MET`. Repeat such a run; do not loosen the check.
 *
 * After each arm the server is started again on the same data directory, with
 * no sentinel, and every key whose last write was acknowledged must read back
 * that value.
 *
 * Environment:
 *
 * - FLUSH_PANIC_ARMS         — comma-separated arms to run (default: both);
 * - FLUSH_PANIC_EXIT_WAIT_MS — how long the test waits for the process to end
 *                              by itself (default: the server's own deadline
 *                              for a failed stop plus a margin, 50 s);
 * - LIVENESS_OUT_DIR         — where each run writes its notes and the server
 *                              streams; unset, nothing is written;
 * - LIVENESS_RUN             — label of the run in those file names.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  SEAM_BOOT_TEXT,
  LagScrape,
  allZero,
  lwwPut,
  requireFaultBinary,
  scrapeLag,
} from './helpers/fault-binary';
import { TestClient } from './helpers/test-client';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  sha256OfFile,
  sleep,
  plainLines,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

type Arm = 'PANIC-THEN-ORDERLY-EXIT' | 'PANIC-IN-SHUTDOWN';
const ALL_ARMS: Arm[] = ['PANIC-THEN-ORDERLY-EXIT', 'PANIC-IN-SHUTDOWN'];

/** Written and flushed while the store is healthy (steps 1 and 2 of both arms). */
const EARLY_MAP = 'flush_panic_early';
const EARLY_KEYS = ['early-0', 'early-1', 'early-2', 'early-3', 'early-4'];
/** The one write of the orderly arm that the dying flush pass holds. */
const LATE_MAP = 'flush_panic_late';
const LATE_KEY = 'late';
/** The stream of the other arm. */
const STREAM_MAP = 'flush_panic_stream';

/** The boot line of a build that has the panic seam and was given a sentinel path. */
const PANIC_SEAM_BOOT_TEXT = 'persisting an entry panics while the panic sentinel file exists';
/** The store's line when one of its background tasks panicked. */
const STORE_FAILED_TEXT =
  'write-behind background task panicked; the store refuses writes from now on';
/** The store's report when a shutdown ran to its end on a failed store. */
const SHUTDOWN_REPORT_TEXT = 'write-behind shutdown on a failed store';
/** The binary's line when it learns of the failure. */
const BINARY_FAILED_TEXT = 'write-behind background task failed; shutting the node down';
/** The binary's line when the shutdown drain itself did not complete. */
const DRAIN_FAILED_TEXT = 'write-behind drain did not complete on a failed store';
const DEADLINE_PASSED_TEXT = 'shutdown on a failed store passed its deadline';

/** An entry is taken by a flush pass once it is this old; fixed in the server binary. */
const WRITE_DELAY_MS = 1_000;
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
/**
 * How long after the last acknowledgement the early writes had all reached the
 * store on the real binary: write delay, one flush tick, and the 600 ms tick
 * that refreshes the gauge.
 */
const FLUSH_OBSERVED_MS = 2_400;
/** Room on top of that for a slower host; the wait ends as soon as the sign is seen. */
const FLUSH_MARGIN_MS = 3_600;
const FLUSH_POLL_MS = 100;
/**
 * The stream may pause for at most 50 ms between an answer and the next send.
 * It does not pause at all beyond yielding, so that a write is in flight at
 * almost every moment and the one that meets the failed store is likely to be
 * one the server had a chance to answer.
 */
const STREAM_PAUSE_MS = 0;
/** The flush task dies one to two seconds into the stream; a stream this long with no refusal is a failure. */
const STREAM_MAX_MS = 15_000;
const LOG_POLL_MS = 5;
const RESTART_STOP_MS = 15_000;
/** Bounds a hung arm: boot, the flush wait, the wait for the exit, and the restart. */
const ARM_TIMEOUT_MS = 180_000;

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

function selectedArms(): Arm[] {
  const raw = process.env.FLUSH_PANIC_ARMS;
  if (!raw) return ALL_ARMS;
  const names = raw
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean);
  for (const name of names) {
    if (!ALL_ARMS.includes(name as Arm)) {
      throw new Error(
        `FLUSH_PANIC_ARMS names an unknown arm ${JSON.stringify(name)}; arms: ${ALL_ARMS.join(', ')}`,
      );
    }
  }
  if (names.length === 0) throw new Error('FLUSH_PANIC_ARMS is set and names no arm');
  return names as Arm[];
}

function exitWaitMs(): number {
  const raw = process.env.FLUSH_PANIC_EXIT_WAIT_MS;
  if (!raw) return FAILURE_DEADLINE_MS + EXIT_WAIT_MARGIN_MS;
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(
      `FLUSH_PANIC_EXIT_WAIT_MS is not a positive whole number of milliseconds: ${raw}`,
    );
  }
  return value;
}

/** One write as the client saw it. */
interface WriteSeen {
  key: string;
  value: unknown;
  sentAt: number;
  /**
   * The type of the frame that answered it, or what happened instead:
   * `connection closed`, `no frame within N ms`, or `not sent: …`.
   */
  answer: string;
  /** The answering frame, when there was one. */
  frame: unknown;
}

/** Sends one write and reports what came back: a frame, the end of the connection, or nothing in time. */
async function writeAndSee(
  client: TestClient,
  id: string,
  mapName: string,
  key: string,
  value: unknown,
): Promise<WriteSeen> {
  const ws = client.ws;
  const sentAt = Date.now();
  if (ws.readyState !== ws.OPEN) {
    return {
      key,
      value,
      sentAt,
      answer: 'not sent: the connection was already closed',
      frame: null,
    };
  }
  client.messages.length = 0;
  client.send({ type: 'CLIENT_OP', payload: lwwPut(id, mapName, key, value) });
  const deadline = sentAt + ACK_WAIT_MS;
  for (;;) {
    // A frame that arrived together with the close is still the answer.
    const frame = client.messages.find((m) => ['OP_ACK', 'OP_REJECTED', 'ERROR'].includes(m.type));
    if (frame) return { key, value, sentAt, answer: frame.type as string, frame };
    if (ws.readyState !== ws.OPEN) {
      return { key, value, sentAt, answer: 'connection closed', frame: null };
    }
    if (Date.now() >= deadline) {
      return { key, value, sentAt, answer: `no frame within ${ACK_WAIT_MS} ms`, frame: null };
    }
    await sleep(LOG_POLL_MS);
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

function field(line: string, name: string): number | null {
  const found = new RegExp(`\\b${name}=(\\d+)\\b`).exec(line);
  return found ? Number(found[1]) : null;
}

interface Observed {
  arm: Arm;
  /** Why the arm has no result, or `null` when its setup held. */
  preconditionNotMet: string | null;
  /** How the process ended by itself, or `null` when it was still running when the wait ran out. */
  exit: BinaryExit | null;
  exitWaitMs: number;
  /** From the test first reading the store's line to the process being gone. */
  exitAfterSeenMs: number | null;
  /** From the sentinel being created to the process being gone. */
  exitAfterSentinelMs: number | null;
  sentinelStillExists: boolean;
  storeFailedLines: string[];
  binaryFailedLines: string[];
  shutdownReportLines: string[];
  drainFailedLines: string[];
  deadlinePassedLines: string[];
  /** Orderly arm: the two fields of the shutdown report. */
  drainedEntries: number | null;
  unpersistedEntries: number | null;
  /** Stream arm: every write of the stream, in order. */
  stream: WriteSeen[];
  /** Stream arm: acknowledged writes sent later than the store's line minus the write delay. */
  ackedInsideWriteDelay: number | null;
  /** Stream arm: the first write that was not acknowledged. */
  refused: WriteSeen | null;
  /** Key to the last acknowledged value, per map. */
  acked: Record<string, Record<string, unknown>>;
  /** What the restarted server returned for the same maps, restricted to the acknowledged keys. */
  readBack: Record<string, Record<string, unknown>> | string;
  restartExit: BinaryExit | null;
  sha256Before: string;
  sha256After: string;
}

async function runArm(arm: Arm): Promise<Observed> {
  const env = requireFaultBinary();
  const waitForExitMs = exitWaitMs();
  const outDir = process.env.LIVENESS_OUT_DIR || null;
  if (outDir) fs.mkdirSync(outDir, { recursive: true });
  const run =
    process.env.LIVENESS_RUN ||
    new Date()
      .toISOString()
      .replace(/[^0-9]/g, '')
      .slice(0, 14);
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-flush-task-panic-'));
  const sentinel = path.join(dataDir, 'panic');
  const startedAt = Date.now();
  const notes: string[] = [];
  const note = (text: string) => {
    notes.push(`${new Date().toISOString()} +${Date.now() - startedAt}ms ${text}`);
  };
  let server: SpawnedBinary | null = null;
  let restarted: SpawnedBinary | null = null;
  let writer: TestClient | null = null;
  let reader: TestClient | null = null;
  let summary = `flush-task-panic arm=${arm} commit=${env.commit} aborted=before the server was launched`;

  const storeEnv: Record<string, string> = {
    STORAGE_BACKEND: 'redb',
    TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS: String(SHUTDOWN_TIMEOUT_MS),
    // Refreshes the lag gauge every 600 ms, which is what step 2 reads.
    TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '6000',
  };

  try {
    // No file exists at the sentinel path yet: the store is healthy until step 3.
    server = await spawnBinary({
      binaryPath: env.binary,
      commit: env.commit,
      label: `flush-task-panic ${arm}`,
      env: { ...storeEnv, TOPGUN_FAULT_STORE_PANIC_FILE: sentinel },
    });
    const launched = server;
    const sha256Before = launched.sha256;
    const abort = (reason: string): never => {
      summary = `flush-task-panic arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=${reason}`;
      throw new Error(`${reason}\n--- stderr ---\n${launched.stderr()}`);
    };
    summary = `flush-task-panic arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=the run did not reach its end`;
    if (launched.port === null) abort('the server did not print PORT=');
    const port = launched.port as number;
    note(`PORT=${port} pid=${launched.process.pid}`);

    const seamDeadline = Date.now() + 2_000;
    const seamLine = () =>
      plainLines(launched.stdout()).find(
        (line) => line.includes(SEAM_BOOT_TEXT) && line.includes(PANIC_SEAM_BOOT_TEXT),
      );
    while (!seamLine() && Date.now() < seamDeadline) await sleep(25);
    if (!seamLine()) {
      abort(
        `the launched binary did not print its "${SEAM_BOOT_TEXT}: ${PANIC_SEAM_BOOT_TEXT}" line: it has no store-panic seam, so nothing it does is about a panicked flush task`,
      );
    }
    note(`seam boot line: ${(seamLine() as string).slice(0, 300)}`);

    // The store's line is looked for on a timer of its own, so that the moment
    // it is first read is recorded while the arm is busy writing.
    let seenAt: number | null = null;
    const watcher = setInterval(() => {
      if (seenAt === null && launched.stdout().includes(STORE_FAILED_TEXT)) seenAt = Date.now();
    }, LOG_POLL_MS);

    const acked: Observed['acked'] = {};
    const recordAck = (mapName: string, seen: WriteSeen) => {
      (acked[mapName] ??= {})[seen.key] = seen.value;
    };
    let preconditionNotMet: string | null = null;
    const stream: WriteSeen[] = [];
    let refused: WriteSeen | null = null;
    let sentinelAt = 0;
    try {
      // One connection for the whole arm. Each newly authenticated connection
      // makes the server write a record of its own, and one opened after the
      // sentinel would put an entry in the queue that this arm does not count.
      writer = await connectWithin(port, 'flush-panic-writer', CONNECT_WAIT_MS);

      // Step 1: several distinct keys, each acknowledged.
      for (const [i, key] of EARLY_KEYS.entries()) {
        const seen = await writeAndSee(writer, `early-${i}`, EARLY_MAP, key, { early: i });
        if (seen.answer !== 'OP_ACK') {
          abort(`step 1: the write to ${key} was answered with ${seen.answer}, not OP_ACK`);
        }
        recordAck(EARLY_MAP, seen);
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
        lastScrape = await scrapeLag(port);
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

      // Step 3: from here on persisting an entry panics. The file is never removed.
      fs.writeFileSync(sentinel, '');
      sentinelAt = Date.now();
      note('step 3: sentinel created: persisting an entry panics');

      if (preconditionNotMet === null && arm === 'PANIC-THEN-ORDERLY-EXIT') {
        // Step 4: ONE more key, and nothing else from this client.
        const seen = await writeAndSee(writer, 'late', LATE_MAP, LATE_KEY, { late: true });
        note(`step 4: the one late write was answered with ${seen.answer}`);
        if (seen.answer !== 'OP_ACK') {
          abort(`step 4: the late write was answered with ${seen.answer}, not OP_ACK`);
        }
        recordAck(LATE_MAP, seen);
      } else if (preconditionNotMet === null) {
        // Step 4: a continuous stream of distinct keys until the first write
        // that is not acknowledged.
        const streamStartedAt = Date.now();
        for (let n = 0; refused === null; n += 1) {
          if (Date.now() - streamStartedAt > STREAM_MAX_MS) {
            abort(
              `step 4: ${n} writes were all acknowledged over ${STREAM_MAX_MS} ms with the sentinel in place: the flush task did not die, or a dead flush task does not stop writes`,
            );
          }
          const seen = await writeAndSee(writer, `stream-${n}`, STREAM_MAP, `stream-${n}`, { n });
          stream.push(seen);
          if (seen.answer === 'OP_ACK') {
            recordAck(STREAM_MAP, seen);
            await sleep(STREAM_PAUSE_MS);
          } else {
            refused = seen;
          }
        }
        note(
          `step 4: ${stream.length} writes sent, ${stream.length - 1} acknowledged; the first not acknowledged (${refused.key}) saw: ${refused.answer} ${JSON.stringify(refused.frame)}`,
        );
      }

      // Step 5: the process ends by itself. The client connection stays open:
      // a server that waits for its clients to leave would not get here.
      const exit =
        preconditionNotMet === null ? await launched.waitForExit(waitForExitMs) : launched.exit();
      const exitAt = Date.now();
      // The watcher's last look may predate the last bytes of the capture.
      if (seenAt === null && launched.stdout().includes(STORE_FAILED_TEXT)) seenAt = exitAt;
      note(`step 5: exit ${JSON.stringify(exit)}; the store's line was first read at ${seenAt}`);

      const out = launched.stdout();
      const storeFailedLines = linesWith(out, STORE_FAILED_TEXT);
      const binaryFailedLines = linesWith(out, BINARY_FAILED_TEXT);
      const shutdownReportLines = linesWith(out, SHUTDOWN_REPORT_TEXT);
      const drainFailedLines = linesWith(out, DRAIN_FAILED_TEXT);
      const deadlinePassedLines = linesWith(out, DEADLINE_PASSED_TEXT);
      for (const line of [
        ...storeFailedLines,
        ...binaryFailedLines,
        ...shutdownReportLines,
        ...drainFailedLines,
        ...deadlinePassedLines,
      ]) {
        note(`server: ${line}`);
      }
      const sentinelStillExists = fs.existsSync(sentinel);

      // Step 6: was the arm's setup what it was meant to be?
      let drainedEntries: number | null = null;
      let unpersistedEntries: number | null = null;
      let ackedInsideWriteDelay: number | null = null;
      if (arm === 'PANIC-THEN-ORDERLY-EXIT' && shutdownReportLines.length > 0) {
        drainedEntries = field(shutdownReportLines[0], 'drained_entries');
        unpersistedEntries = field(shutdownReportLines[0], 'unpersisted_entries');
        if (preconditionNotMet === null && (drainedEntries !== 0 || unpersistedEntries !== 1)) {
          preconditionNotMet = `the shutdown report carries drained_entries=${drainedEntries} unpersisted_entries=${unpersistedEntries}; the arm needs drained_entries=0 (nothing queued outside the dead batch) and unpersisted_entries=1 (the dead batch is the one late write)`;
        }
      }
      if (arm === 'PANIC-IN-SHUTDOWN' && seenAt !== null) {
        const from = seenAt - WRITE_DELAY_MS;
        ackedInsideWriteDelay = stream.filter(
          (seen) => seen.answer === 'OP_ACK' && seen.sentAt > from,
        ).length;
        note(
          `step 6: ${ackedInsideWriteDelay} acknowledged writes were sent later than ${WRITE_DELAY_MS} ms before the store's line was read`,
        );
        if (preconditionNotMet === null && ackedInsideWriteDelay === 0) {
          preconditionNotMet = `no acknowledged write of the stream was sent later than ${WRITE_DELAY_MS} ms before the store's line was read, so nothing is known to have been queued outside the dead batch`;
        }
      }

      // Afterwards: the same data directory, no sentinel, every acknowledged key.
      let readBack: Observed['readBack'] = 'NOT OBSERVED: the process did not end by itself';
      let restartExit: BinaryExit | null = null;
      if (exit !== null) {
        if (writer.ws.readyState !== writer.ws.CLOSED) writer.ws.terminate();
        restarted = await spawnBinary({
          binaryPath: env.binary,
          commit: env.commit,
          label: `flush-task-panic ${arm} restart`,
          env: storeEnv,
        });
        if (restarted.port === null) {
          readBack = `NOT OBSERVED: the restarted server did not print PORT= (exit ${JSON.stringify(restarted.exit())}): ${restarted.stderr().slice(0, 500)}`;
        } else {
          try {
            reader = await connectWithin(restarted.port, 'flush-panic-reader', CONNECT_WAIT_MS);
            const read: Record<string, Record<string, unknown>> = {};
            for (const mapName of Object.keys(acked)) {
              const rows = await readMap(reader, mapName);
              read[mapName] = {};
              for (const key of Object.keys(acked[mapName])) read[mapName][key] = rows[key];
            }
            readBack = read;
          } catch (err) {
            readBack = `NOT OBSERVED: ${(err as Error).message}`;
          }
          if (reader) reader.ws.terminate();
          restartExit = await restarted.stop('SIGTERM', RESTART_STOP_MS);
        }
        note(
          `restart: read back ${
            typeof readBack === 'string'
              ? readBack
              : Object.entries(readBack)
                  .map(([mapName, rows]) => `${mapName}:${Object.keys(rows).length} keys`)
                  .join(' ')
          }; exit on SIGTERM ${JSON.stringify(restartExit)}`,
        );
      }

      const sha256After = sha256OfFile(env.binary);
      const exitAfterSeenMs = exit !== null && seenAt !== null ? exitAt - seenAt : null;
      const exitAfterSentinelMs = exit !== null ? exitAt - sentinelAt : null;
      const readBackOk =
        typeof readBack !== 'string' && JSON.stringify(readBack) === JSON.stringify(acked);
      summary =
        `flush-task-panic arm=${arm} sha256_before=${sha256Before} sha256_after=${sha256After} commit=${env.commit} ` +
        `precondition=${preconditionNotMet === null ? 'met' : 'NOT MET'} exit=${JSON.stringify(exit)} ` +
        `exit_after_seen_ms=${exitAfterSeenMs} exit_after_sentinel_ms=${exitAfterSentinelMs} ` +
        `sentinel_still_exists=${sentinelStillExists} store_failed_lines=${storeFailedLines.length} ` +
        `binary_failed_lines=${binaryFailedLines.length} shutdown_report_lines=${shutdownReportLines.length} ` +
        `drain_failed_lines=${drainFailedLines.length} deadline_passed_lines=${deadlinePassedLines.length} ` +
        `drained_entries=${drainedEntries} unpersisted_entries=${unpersistedEntries} ` +
        `stream_writes=${stream.length} acked_inside_write_delay=${ackedInsideWriteDelay} ` +
        `refused_write_saw=${JSON.stringify(refused ? refused.answer : null)} ` +
        `acked_keys=${Object.values(acked).reduce((sum, rows) => sum + Object.keys(rows).length, 0)} ` +
        `read_back=${readBackOk ? 'ok' : 'fail'} restart_exit=${JSON.stringify(restartExit)}`;

      return {
        arm,
        preconditionNotMet,
        exit,
        exitWaitMs: waitForExitMs,
        exitAfterSeenMs,
        exitAfterSentinelMs,
        sentinelStillExists,
        storeFailedLines,
        binaryFailedLines,
        shutdownReportLines,
        drainFailedLines,
        deadlinePassedLines,
        drainedEntries,
        unpersistedEntries,
        stream,
        ackedInsideWriteDelay,
        refused,
        acked,
        readBack,
        restartExit,
        sha256Before,
        sha256After,
      };
    } finally {
      clearInterval(watcher);
    }
  } finally {
    for (const client of [writer, reader]) {
      if (client && client.ws.readyState !== client.ws.CLOSED) client.ws.terminate();
    }
    if (server) await server.stop('SIGKILL', 5_000);
    if (restarted) await restarted.stop('SIGKILL', 5_000);
    // Straight to stderr: the suite silences console.log, and this line is the record of the run.
    process.stderr.write(`${summary}\n`);
    if (outDir) {
      const base = path.join(outDir, `flush-task-panic-${arm}-${run}`);
      fs.writeFileSync(`${base}.notes.txt`, `${notes.join('\n')}\n${summary}\n`);
      fs.writeFileSync(`${base}.stdout.txt`, server ? server.stdout() : '');
      fs.writeFileSync(`${base}.stderr.txt`, server ? server.stderr() : '');
      fs.writeFileSync(`${base}.restart.stdout.txt`, restarted ? restarted.stdout() : '');
      fs.writeFileSync(`${base}.restart.stderr.txt`, restarted ? restarted.stderr() : '');
    }
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

/** What both arms assert about the end of the process, apart from the code itself. */
function endOfProcess(o: Observed) {
  return {
    exit: o.exit,
    exitedInsideTheDeadline: o.exitAfterSeenMs !== null && o.exitAfterSeenMs < FAILURE_DEADLINE_MS,
    sentinelStillExists: o.sentinelStillExists,
    storeFailedLines: o.storeFailedLines.length,
    shutdownReportLines: o.shutdownReportLines.length,
    readBackAfterRestart: o.readBack,
    restartExitOnSigterm: o.restartExit,
  };
}

describe('Integration: a server whose flush task panicked ends by itself with a non-zero exit code', () => {
  const titles: Record<Arm, string> = {
    'PANIC-THEN-ORDERLY-EXIT':
      'PANIC-THEN-ORDERLY-EXIT: with nothing queued outside the dead batch the process exits 70 by itself, and a restart reads every acknowledged key',
    'PANIC-IN-SHUTDOWN':
      'PANIC-IN-SHUTDOWN: with younger writes still queued the shutdown drain panics too and the process exits 71, and a restart reads every acknowledged key',
  };
  for (const arm of selectedArms()) {
    test(
      titles[arm],
      async () => {
        const o = await runArm(arm);
        if (o.preconditionNotMet !== null) {
          throw new ScenarioPreconditionError(arm, o.preconditionNotMet);
        }
        if (o.exit === null) {
          throw new Error(
            `arm ${arm}: the process did not exit by itself within ${o.exitWaitMs} ms of its flush task being made to panic, with the client connection open (store's line seen: ${o.storeFailedLines.length}, binary's line seen: ${o.binaryFailedLines.length}). It was killed by the test.`,
          );
        }
        if (arm === 'PANIC-THEN-ORDERLY-EXIT') {
          expect({
            ...endOfProcess(o),
            binaryFailedLinesWithExitCode70: o.binaryFailedLines.filter((line) =>
              /\bexit_code=70\b/.test(line),
            ).length,
            drainedEntries: o.drainedEntries,
            unpersistedEntries: o.unpersistedEntries,
          }).toEqual({
            exit: { code: 70, signal: null },
            exitedInsideTheDeadline: true,
            sentinelStillExists: true,
            storeFailedLines: 1,
            shutdownReportLines: 1,
            readBackAfterRestart: o.acked,
            restartExitOnSigterm: { code: 0, signal: null },
            binaryFailedLinesWithExitCode70: 1,
            drainedEntries: 0,
            unpersistedEntries: 1,
          });
          return;
        }
        expect({
          ...endOfProcess(o),
          ackedWritesInsideTheWriteDelay: (o.ackedInsideWriteDelay as number) >= 1,
          refusedWriteWasAcknowledged: o.refused === null || o.refused.answer === 'OP_ACK',
        }).toEqual({
          exit: { code: 71, signal: null },
          exitedInsideTheDeadline: true,
          sentinelStillExists: true,
          storeFailedLines: 1,
          // The drain panicked before the shutdown reached any of its returns.
          shutdownReportLines: 0,
          readBackAfterRestart: o.acked,
          restartExitOnSigterm: { code: 0, signal: null },
          ackedWritesInsideTheWriteDelay: true,
          refusedWriteWasAcknowledged: false,
        });
      },
      ARM_TIMEOUT_MS,
    );
  }
});
