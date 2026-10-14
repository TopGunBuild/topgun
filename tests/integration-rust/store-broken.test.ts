/**
 * Integration: a server whose embedded redb store reports an I/O error stops
 * in order with exit code 72, and a restart on a healed disk reads back every
 * acknowledged write.
 *
 * After its first I/O error a redb handle refuses every later write
 * transaction, also once the cause is gone; only a fresh handle — a new
 * process — can store again. A write is acknowledged once it is in the
 * write-ahead log, so the acknowledged writes the broken handle could not
 * store are replayed from the WAL by the next process. This test makes the
 * handle break under a running server and checks what a process supervisor
 * and an operator would see: the process ends by itself with code 72, the
 * store and the binary say why, and after a restart every acknowledged write
 * is readable (`TG-WB-010`).
 *
 * The I/O error comes from a server built with the `fault-injection` cargo
 * feature: while a file exists at the path named by TOPGUN_FAULT_REDB_IO_FILE,
 * redb's writes to its file answer ENOSPC ("No space left on device"); reads
 * pass. The binary is given by path (FAULT_SERVER_BINARY) together with the
 * commit it was built from (FAULT_SERVER_COMMIT). With either missing the test
 * fails; it never skips and never falls back to another binary.
 *
 * Two arms, one data directory each:
 *
 * - LATCH-THEN-ORDERLY-EXIT — a few keys are written and reach the store, the
 *   sentinel is created, one more key is written and acknowledged. The flush
 *   of that key meets the I/O error, the store fails, and the process exits 72
 *   by itself inside the server's own deadline for a failed stop. The sentinel
 *   is removed ("the disk has room again") and a restart on the same directory
 *   reads every acknowledged key.
 * - STILL-FULL-AT-RESTART — the same up to the exit 72, but the sentinel stays
 *   for the restart: opening the store writes, so the restarted process exits
 *   1 by itself before it is ready and names the I/O error. Then the sentinel
 *   is removed and a third start reads every acknowledged key.
 *
 * Each arm checks that its own setup held (its precondition) before it says
 * anything. A precondition that did not hold is an error of the scenario — the
 * run is neither a pass nor a failure of the arm — and is thrown as
 * `ScenarioPreconditionError`, whose message starts with
 * `SCENARIO PRECONDITION NOT MET`. Repeat such a run; do not loosen the check.
 *
 * Environment:
 *
 * - STORE_BROKEN_EXIT_WAIT_MS — how long the test waits for the process to end
 *                               by itself (default: the server's own deadline
 *                               for a failed stop plus a margin, 50 s);
 * - LIVENESS_OUT_DIR          — where each run writes its notes and the server
 *                               streams; unset, nothing is written;
 * - LIVENESS_RUN              — label of the run in those file names.
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
  within,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

type Arm = 'LATCH-THEN-ORDERLY-EXIT' | 'STILL-FULL-AT-RESTART';

/** Written and flushed while the store is healthy. */
const EARLY_MAP = 'store_broken_early';
const EARLY_KEYS = ['early-0', 'early-1', 'early-2', 'early-3', 'early-4'];
/** The one write whose flush meets the I/O error. */
const LATE_MAP = 'store_broken_late';
const LATE_KEY = 'late';

/** The boot line of a build that has the redb I/O seam and was given a sentinel path. */
const REDB_SEAM_BOOT_TEXT = 'redb writes answer ENOSPC while the sentinel file exists';
/** The store's line when it classifies an error as a broken store. */
const STORE_BROKEN_TEXT =
  'write-behind store: the durable store is broken and stores nothing more in this process; writes are refused from now on and the node stops';
/** The store's line when its shutdown drain stops at a broken store. */
const DRAIN_STOPPED_TEXT =
  'write-behind shutdown drain stopped at a broken store: the entries it did not store stay un-resolved and replay from the WAL at the next start';
/** The binary's line when it learns that the store is broken. */
const BINARY_BROKEN_TEXT = 'the durable store is broken; shutting the node down';
/** The store's line for a panicked background task: a broken store must not be reported as one. */
const PANIC_TEXT = 'write-behind background task panicked';
/** What the restarted process names when it cannot open the store on a full disk. */
const NO_SPACE_TEXT = 'No space left on device';

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
const SEAM_LINE_WAIT_MS = 2_000;
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
/** How long a restart on a still-full disk may take to end by itself once it has not printed its port. */
const STILL_FULL_EXIT_WAIT_MS = 15_000;
const RESTART_STOP_MS = 15_000;
/** Bounds a hung arm: boot, the flush wait, the wait for the exit, and up to two restarts. */
const ARM_TIMEOUT_MS = 240_000;

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

interface Observed {
  arm: Arm;
  /** Why the arm has no result, or `null` when its setup held. */
  preconditionNotMet: string | null;
  lateWriteAnswer: string;
  /** How the process ended by itself, or `null` when it was still running when the wait ran out. */
  exit: BinaryExit | null;
  exitWaitMs: number;
  /** From the sentinel being created to the process being gone. */
  exitAfterSentinelMs: number | null;
  storeBrokenLines: string[];
  drainStoppedLines: string[];
  binaryBrokenLines: string[];
  panicLines: number;
  /** Arm 2 only: the restart with the sentinel still present. */
  stillFull: { port: number | null; exit: BinaryExit | null; noSpaceNamed: boolean } | null;
  /** Key to the last acknowledged value, per map. */
  acked: Record<string, Record<string, unknown>>;
  /** What the healed restart returned for the same maps, restricted to the acknowledged keys. */
  readBack: Record<string, Record<string, unknown>> | string;
  healedExitOnSigterm: BinaryExit | null;
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
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-store-broken-'));
  const sentinel = path.join(dataDir, 'disk-full');
  const startedAt = Date.now();
  const notes: string[] = [];
  const note = (text: string) => {
    notes.push(`${new Date().toISOString()} +${Date.now() - startedAt}ms ${text}`);
  };
  let server: SpawnedBinary | null = null;
  let stillFullServer: SpawnedBinary | null = null;
  let healed: SpawnedBinary | null = null;
  let writer: TestClient | null = null;
  let reader: TestClient | null = null;
  let summary = `store-broken arm=${arm} commit=${env.commit} aborted=before the server was launched`;

  const storeEnv: Record<string, string> = {
    STORAGE_BACKEND: 'redb',
    TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS: String(SHUTDOWN_TIMEOUT_MS),
    // Refreshes the lag gauge every 600 ms, which is what the flush wait reads.
    TOPGUN_WAL_WATERMARK_STALL_BOUND_MS: '6000',
    // Every start of the arm is given the same sentinel path; whether a file
    // exists there is what decides if redb writes are refused.
    TOPGUN_FAULT_REDB_IO_FILE: sentinel,
  };

  try {
    // No file exists at the sentinel path yet: the disk "has room" until the sentinel is created.
    server = await spawnBinary({
      binaryPath: env.binary,
      commit: env.commit,
      label: `store-broken ${arm}`,
      env: storeEnv,
    });
    const launched = server;
    const sha256Before = launched.sha256;
    const abort = (reason: string): never => {
      summary = `store-broken arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=${reason}`;
      throw new Error(`${reason}\n--- stderr ---\n${launched.stderr()}`);
    };
    summary = `store-broken arm=${arm} sha256=${sha256Before} commit=${env.commit} aborted=the run did not reach its end`;
    if (launched.port === null) abort('the server did not print PORT=');
    const port = launched.port as number;
    note(`PORT=${port}`);

    const seamDeadline = Date.now() + SEAM_LINE_WAIT_MS;
    const seamLine = () =>
      plainLines(launched.stdout()).find(
        (line) => line.includes(SEAM_BOOT_TEXT) && line.includes(REDB_SEAM_BOOT_TEXT),
      );
    while (!seamLine() && Date.now() < seamDeadline) await sleep(25);
    if (!seamLine()) {
      abort(
        `the launched binary did not print its "${SEAM_BOOT_TEXT}: ${REDB_SEAM_BOOT_TEXT}" line: it has no redb I/O seam, so nothing it does is about a broken store`,
      );
    }
    note(`seam boot line seen`);

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

    // Step 3: from here on redb's writes to its file answer ENOSPC.
    fs.writeFileSync(sentinel, '');
    const sentinelAt = Date.now();
    note('step 3: sentinel created: redb writes answer ENOSPC');

    // Step 4: ONE more key. Its WAL frame is written before the ack; its
    // flush to redb is what meets the I/O error.
    let lateWriteAnswer = 'not sent: the precondition did not hold';
    if (preconditionNotMet === null) {
      lateWriteAnswer = await writeAndSee(writer, 'late', LATE_MAP, LATE_KEY, { late: true });
      note(`step 4: the late write was answered with ${lateWriteAnswer}`);
      if (lateWriteAnswer === 'OP_ACK') (acked[LATE_MAP] ??= {})[LATE_KEY] = { late: true };
    }

    // Step 5: the process ends by itself, with the client connection still open.
    const exit =
      preconditionNotMet === null ? await launched.waitForExit(waitForExitMs) : launched.exit();
    const exitAt = Date.now();
    const exitAfterSentinelMs = exit !== null ? exitAt - sentinelAt : null;
    note(`step 5: exit ${JSON.stringify(exit)} ${exitAfterSentinelMs} ms after the sentinel`);
    if (exit === null) await launched.stop('SIGKILL', 5_000);

    const out = launched.stdout();
    const storeBrokenLines = linesWith(out, STORE_BROKEN_TEXT);
    const drainStoppedLines = linesWith(out, DRAIN_STOPPED_TEXT);
    const binaryBrokenLines = linesWith(out, BINARY_BROKEN_TEXT);
    const panicLines = linesWith(out, PANIC_TEXT).length;
    for (const line of [...storeBrokenLines, ...drainStoppedLines, ...binaryBrokenLines]) {
      note(`server: ${line}`);
    }
    if (writer.ws.readyState !== writer.ws.CLOSED) writer.ws.terminate();

    let stillFull: Observed['stillFull'] = null;
    let readBack: Observed['readBack'] = 'NOT OBSERVED: the process did not end by itself';
    let healedExitOnSigterm: BinaryExit | null = null;
    if (exit !== null) {
      if (arm === 'STILL-FULL-AT-RESTART') {
        // The sentinel stays: opening the store must write, and cannot.
        stillFullServer = await spawnBinary({
          binaryPath: env.binary,
          commit: env.commit,
          label: `store-broken ${arm} restart with the sentinel present`,
          env: storeEnv,
        });
        const stillFullPort = stillFullServer.port;
        const stillFullExit =
          stillFullPort === null
            ? await stillFullServer.waitForExit(STILL_FULL_EXIT_WAIT_MS)
            : stillFullServer.exit();
        const stillFullOutput = `${stillFullServer.stdout()}\n${stillFullServer.stderr()}`;
        stillFull = {
          port: stillFullPort,
          exit: stillFullExit,
          noSpaceNamed: stillFullServer.stderr().includes(NO_SPACE_TEXT),
        };
        note(`restart with the sentinel present: ${JSON.stringify(stillFull)}`);
        for (const line of plainLines(stillFullOutput).filter((l) => l.includes(NO_SPACE_TEXT))) {
          note(`restart with the sentinel present: ${line}`);
        }
        await stillFullServer.stop('SIGKILL', 5_000);
        if (stillFullPort !== null || stillFullExit === null || stillFullExit.code !== 1) {
          preconditionNotMet = `the restart with the sentinel present did not exit 1 by itself before it was ready (port ${stillFullPort}, exit ${JSON.stringify(stillFullExit)})`;
        }
      }

      // The disk has room again: the same data directory, no sentinel.
      fs.rmSync(sentinel, { force: true });
      healed = await spawnBinary({
        binaryPath: env.binary,
        commit: env.commit,
        label: `store-broken ${arm} healed restart`,
        env: storeEnv,
      });
      if (healed.port === null) {
        readBack = `NOT OBSERVED: the healed restart did not print PORT= (exit ${JSON.stringify(healed.exit())}): ${healed.stderr().slice(0, 500)}`;
      } else {
        try {
          reader = await connectWithin(healed.port, 'store-broken-reader', CONNECT_WAIT_MS);
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
      typeof readBack !== 'string' && JSON.stringify(readBack) === JSON.stringify(acked);
    summary =
      `store-broken arm=${arm} sha256_before=${sha256Before} sha256_after=${sha256After} commit=${env.commit} ` +
      `precondition=${preconditionNotMet === null ? 'met' : 'NOT MET'} flushed_after_ms=${flushedAfterMs} ` +
      `late_write=${lateWriteAnswer} exit=${JSON.stringify(exit)} exit_after_sentinel_ms=${exitAfterSentinelMs} ` +
      `store_broken_lines=${storeBrokenLines.length} drain_stopped_lines=${drainStoppedLines.length} ` +
      `binary_broken_lines=${binaryBrokenLines.length} panic_lines=${panicLines} ` +
      `still_full_restart=${JSON.stringify(stillFull)} ` +
      `acked_keys=${Object.values(acked).reduce((sum, rows) => sum + Object.keys(rows).length, 0)} ` +
      `read_back=${readBackOk ? 'ok' : 'fail'} healed_exit_on_sigterm=${JSON.stringify(healedExitOnSigterm)}`;

    return {
      arm,
      preconditionNotMet,
      lateWriteAnswer,
      exit,
      exitWaitMs: waitForExitMs,
      exitAfterSentinelMs,
      storeBrokenLines,
      drainStoppedLines,
      binaryBrokenLines,
      panicLines,
      stillFull,
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
    for (const proc of [server, stillFullServer, healed]) {
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
        `${base}.still-full.stderr.txt`,
        stillFullServer ? stillFullServer.stderr() : '',
      );
      fs.writeFileSync(`${base}.healed.stdout.txt`, healed ? healed.stdout() : '');
      fs.writeFileSync(`${base}.healed.stderr.txt`, healed ? healed.stderr() : '');
    }
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

/** Fails the arm with a named observation when the process had to be ended by the test. */
function requireExitByItself(o: Observed): void {
  if (o.exit === null) {
    throw new Error(
      `arm ${o.arm}: the process did not exit by itself within ${o.exitWaitMs} ms of its redb store being made to refuse writes, with the client connection open (late write: ${o.lateWriteAnswer}; store's line seen: ${o.storeBrokenLines.length}, drain line seen: ${o.drainStoppedLines.length}, binary's line seen: ${o.binaryBrokenLines.length}). It was killed by the test.`,
    );
  }
}

/** What both arms assert about the first process. */
function firstProcess(o: Observed) {
  return {
    lateWriteAnswer: o.lateWriteAnswer,
    exit: o.exit,
    exitedInsideTheDeadline:
      o.exitAfterSentinelMs !== null && o.exitAfterSentinelMs < FAILURE_DEADLINE_MS,
    storeBrokenLines: o.storeBrokenLines.length,
    storeBrokenLineNamesItsCause: o.storeBrokenLines.filter(
      (line) => line.includes('cause="io"') && line.includes('io_kind="StorageFull"'),
    ).length,
    drainStoppedLines: o.drainStoppedLines.length,
    drainStoppedLineNamesItsCauseAndCount: o.drainStoppedLines.filter(
      (line) => /\bcause="\w+"/.test(line) && /\babandoned_entries=[1-9]\d*\b/.test(line),
    ).length,
    binaryBrokenLines: o.binaryBrokenLines.length,
    binaryBrokenLinesWithExitCode72: o.binaryBrokenLines.filter(
      (line) => /\bexit_code=72\b/.test(line) && line.includes('cause="io"'),
    ).length,
    panicLines: o.panicLines,
  };
}

const EXPECTED_FIRST_PROCESS = {
  lateWriteAnswer: 'OP_ACK',
  exit: { code: 72, signal: null },
  exitedInsideTheDeadline: true,
  storeBrokenLines: 1,
  storeBrokenLineNamesItsCause: 1,
  drainStoppedLines: 1,
  drainStoppedLineNamesItsCauseAndCount: 1,
  binaryBrokenLines: 1,
  binaryBrokenLinesWithExitCode72: 1,
  panicLines: 0,
};

describe('Integration: a server whose redb store is broken ends by itself with exit code 72', () => {
  test(
    'arm 1 LATCH-THEN-ORDERLY-EXIT',
    async () => {
      const o = await runArm('LATCH-THEN-ORDERLY-EXIT');
      if (o.preconditionNotMet !== null) {
        throw new ScenarioPreconditionError(o.arm, o.preconditionNotMet);
      }
      requireExitByItself(o);
      expect({
        ...firstProcess(o),
        readBackAfterRestart: o.readBack,
        healedExitOnSigterm: o.healedExitOnSigterm,
        sha256BeforeEqualsAfter: o.sha256Before === o.sha256After,
      }).toEqual({
        ...EXPECTED_FIRST_PROCESS,
        readBackAfterRestart: o.acked,
        healedExitOnSigterm: { code: 0, signal: null },
        sha256BeforeEqualsAfter: true,
      });
      // Every key acknowledged before the failure, and the one whose flush met it.
      expect(Object.keys(o.acked[EARLY_MAP] ?? {})).toEqual(EARLY_KEYS);
      expect(Object.keys(o.acked[LATE_MAP] ?? {})).toEqual([LATE_KEY]);
    },
    ARM_TIMEOUT_MS,
  );

  test(
    'arm 2 STILL-FULL-AT-RESTART',
    async () => {
      const o = await runArm('STILL-FULL-AT-RESTART');
      if (o.preconditionNotMet !== null) {
        throw new ScenarioPreconditionError(o.arm, o.preconditionNotMet);
      }
      requireExitByItself(o);
      expect({
        ...firstProcess(o),
        restartWithTheSentinelPresent: o.stillFull,
        readBackAfterTheThirdStart: o.readBack,
        healedExitOnSigterm: o.healedExitOnSigterm,
        sha256BeforeEqualsAfter: o.sha256Before === o.sha256After,
      }).toEqual({
        ...EXPECTED_FIRST_PROCESS,
        restartWithTheSentinelPresent: {
          port: null,
          exit: { code: 1, signal: null },
          noSpaceNamed: true,
        },
        readBackAfterTheThirdStart: o.acked,
        healedExitOnSigterm: { code: 0, signal: null },
        sha256BeforeEqualsAfter: true,
      });
      expect(Object.keys(o.acked[EARLY_MAP] ?? {})).toEqual(EARLY_KEYS);
      expect(Object.keys(o.acked[LATE_MAP] ?? {})).toEqual([LATE_KEY]);
    },
    ARM_TIMEOUT_MS,
  );
});
