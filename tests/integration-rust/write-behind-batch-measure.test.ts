/**
 * Measurement: the write-behind drain and the WAL replay at start, before and
 * after batching, on this machine.
 *
 * Two server binaries run out of process, each given by path: the base (a
 * build of 6a030bb6, which stores one entry and replays one frame per durable
 * write) and the binary under test. Six runs in the order b,n,n,b,b,n; the
 * three base runs are the base-against-base arm the noise rule reads. Every
 * run starts on a fresh directory with `TOPGUN_WAL_FSYNC_POLICY=per_op`, a
 * flush interval of one hour (no flush pass runs before the stop) and a
 * shutdown timeout of ten minutes (the base drain finishes). Per run:
 *
 * - D: write N distinct keys, send SIGTERM, time it until the process is gone;
 *   D = N / that time. F = the size of `engine.redb` after the exit. The run
 *   counts only if the drain was complete (base: exit 0 and no `Shutdown drain
 *   timed out`; new: exit 0 and `rollback hints written` with `drained=true`)
 *   and the next start replays no frame. On the new arm that start then
 *   writes the same N keys again and is stopped with SIGTERM; F2 = the size of
 *   `engine.redb` after that drain, which must end with `drained=true`.
 * - S: write N more keys, `kill -9`, start again: S = spawn to `PORT=`. The run
 *   counts only if the writes went through one client session and that start
 *   replayed exactly N + 1 frames: the N writes and the one device-credential
 *   record the server mints, through the WAL, for a session that presents no
 *   device token (both binaries do). The D session's record is drained with D.
 * - P (new arm only): a fresh directory with a 60 s flush interval; write N
 *   keys, wait for the passes to empty the buffer, read the p99 of
 *   `topgun_write_behind_flush_pass_seconds` in the first scrape that shows it.
 *
 * An invalid run is repeated once; a second invalid run at the same position
 * fails the suite. The gate is pre-registered: D_new >= 20 x D_base,
 * S_new <= 0.25 x S_base, F_new <= 2.25 x F_base (arm values = the median of
 * three runs), F2 <= F on every new run, and P99_new <= 1 s on the worst new
 * run. F is bounded by 2.25x, not by a tighter share: while redb's file is
 * below one region it grows by doubling its usable space, not the file
 * (`PageManager::grow` in redb's `tree_store/page_store/page_manager.rs`;
 * the header and the region layout come on top, so one step is a little over
 * 2x of the file), and a commit of a whole pass needs
 * its copied pages next to the pages the previous commit freed, which are
 * reusable only after the next commit. On a small store that peak can cross
 * one doubling step that one-entry commits stay below; F2 <= F shows that the
 * space is reused rather than grown on every drain. A ratio metric whose
 * three base runs spread more than 1.10 (max / min) re-runs the whole series
 * once; in the re-run it is judged by arm values if its spread is now within
 * 1.10, else by the worst pairing (S, F: max new / min base; D: min new / max
 * base). Every run, its checks and the verdicts go to one JSON file.
 *
 * Variables, all required — the suite fails, never skips, without one:
 * MEASURE_BASE_SERVER_BINARY, MEASURE_BASE_SERVER_COMMIT (starts with
 * 6a030bb6), RUST_SERVER_BINARY, MEASURE_NEW_SERVER_COMMIT (equals
 * `git rev-parse HEAD` of this checkout). MEASURE_OUT_FILE names the JSON
 * file (default: a file in the system temporary directory).
 */

import * as child_process from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import {
  spawnBinary,
  connectWithin,
  firstFrame,
  plainLines,
  logField,
  sleep,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';
import { scrapeGauge } from './helpers/fault-binary';

const N = 9_900;
const OPS_PER_BATCH = 100;
const BASE_COMMIT_PREFIX = '6a030bb6';
const ORDER: Arm[] = ['b', 'n', 'n', 'b', 'b', 'n'];

const FLUSH_INTERVAL_NO_TICK_MS = '3600000';
const FLUSH_INTERVAL_P_MS = '60000';
const SHUTDOWN_TIMEOUT_MS = '600000';
/** The base drain (~162 s) and replay (~45 s) fit well inside these. */
const DRAIN_EXIT_WAIT_MS = 700_000;
const PORT_WAIT_MS = 300_000;
const BATCH_ACK_WAIT_MS = 120_000;
const P_WAIT_MS = 600_000;
/** Six runs, each up to a ~162 s base drain and a ~45 s base replay plus the fills, and one re-run of the series. */
const SUITE_TIMEOUT_MS = 3 * 60 * 60 * 1000;

const GATE = { dMin: 20, sMax: 0.25, fMax: 2.25, p99MaxS: 1 };
const NOISE_BOUND = 1.1;

type Arm = 'b' | 'n';
type Metric = 'D' | 'S' | 'F';

interface Binaries {
  base: { path: string; commit: string };
  next: { path: string; commit: string };
}

interface RunRecord {
  position: number;
  arm: Arm;
  attempt: number;
  dir: string;
  valid: boolean;
  invalidReason?: string;
  /** Entries per second of the SIGTERM drain. */
  D?: number;
  drainSeconds?: number;
  /** Seconds from spawn to `PORT=` of the start that replays N frames. */
  S?: number;
  /** Bytes of `engine.redb` after the drain's exit. */
  F?: number;
  /** Bytes of `engine.redb` after the same keys are written and drained once more (new arm only). */
  F2?: number;
  /** p99 of one flush pass, seconds (new arm only). */
  P99?: number;
  checks: Record<string, unknown>;
}

function requireBinaries(): Binaries {
  const missing = [
    'MEASURE_BASE_SERVER_BINARY',
    'MEASURE_BASE_SERVER_COMMIT',
    'RUST_SERVER_BINARY',
    'MEASURE_NEW_SERVER_COMMIT',
  ].filter((name) => !process.env[name]);
  if (missing.length > 0) {
    throw new Error(
      `${missing.join(', ')} ${missing.length > 1 ? 'are' : 'is'} not set. The measurement needs a release ` +
        `build of ${BASE_COMMIT_PREFIX} in its own worktree and target directory ` +
        '(MEASURE_BASE_SERVER_BINARY, MEASURE_BASE_SERVER_COMMIT) and a release build of this checkout ' +
        '(RUST_SERVER_BINARY, MEASURE_NEW_SERVER_COMMIT = `git rev-parse HEAD`).',
    );
  }
  const env = (name: string) => process.env[name] as string;
  for (const name of ['MEASURE_BASE_SERVER_BINARY', 'RUST_SERVER_BINARY']) {
    try {
      if (!fs.statSync(env(name)).isFile()) throw new Error('not a regular file');
      fs.accessSync(env(name), fs.constants.X_OK);
    } catch (err) {
      throw new Error(
        `${name} does not name an executable file (${env(name)}: ${(err as Error).message})`,
      );
    }
  }
  if (!env('MEASURE_BASE_SERVER_COMMIT').startsWith(BASE_COMMIT_PREFIX)) {
    throw new Error(
      `MEASURE_BASE_SERVER_COMMIT is ${env('MEASURE_BASE_SERVER_COMMIT')}, not a commit starting ${BASE_COMMIT_PREFIX}`,
    );
  }
  const head = child_process
    .execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path.resolve(__dirname, '..', '..') })
    .toString()
    .trim();
  if (env('MEASURE_NEW_SERVER_COMMIT') !== head) {
    throw new Error(
      `MEASURE_NEW_SERVER_COMMIT is ${env('MEASURE_NEW_SERVER_COMMIT')}, but this checkout is at ${head}`,
    );
  }
  return {
    base: { path: env('MEASURE_BASE_SERVER_BINARY'), commit: env('MEASURE_BASE_SERVER_COMMIT') },
    next: { path: env('RUST_SERVER_BINARY'), commit: env('MEASURE_NEW_SERVER_COMMIT') },
  };
}

function storeEnv(dir: string, flushIntervalMs: string): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_WRITEBEHIND_FLUSH_INTERVAL_MS: flushIntervalMs,
    TOPGUN_WRITEBEHIND_SHUTDOWN_TIMEOUT_MS: SHUTDOWN_TIMEOUT_MS,
  };
}

async function start(
  binaries: Binaries,
  arm: Arm,
  dir: string,
  label: string,
  flushIntervalMs = FLUSH_INTERVAL_NO_TICK_MS,
): Promise<{ server: SpawnedBinary; startSeconds: number }> {
  const binary = arm === 'b' ? binaries.base : binaries.next;
  const began = process.hrtime.bigint();
  const server = await spawnBinary({
    binaryPath: binary.path,
    commit: binary.commit,
    label,
    env: storeEnv(dir, flushIntervalMs),
    portTimeoutMs: PORT_WAIT_MS,
  });
  const startSeconds = Number(process.hrtime.bigint() - began) / 1e9;
  if (server.port === null) {
    await server.stop('SIGKILL');
    throw new Error(`${label} printed no PORT=: ${server.stderr().slice(-2000)}`);
  }
  return { server, startSeconds };
}

/** Client sessions `fill` has opened; each one adds a device-credential frame to the WAL. */
let sessionsOpened = 0;

/** Writes `count` distinct keys `{prefix}{i}` of ~100-byte values in OP_BATCH frames, each acknowledged. */
async function fill(port: number, prefix: string, count: number): Promise<void> {
  const client = await connectWithin(port, `measure-${prefix}`, 30_000);
  sessionsOpened += 1;
  try {
    for (let first = 0; first < count; first += OPS_PER_BATCH) {
      const ops = [];
      for (let i = first; i < Math.min(first + OPS_PER_BATCH, count); i++) {
        ops.push({
          id: `${prefix}${i}`,
          mapName: 'measure',
          opType: 'PUT',
          key: `${prefix}${i}`,
          record: {
            value: { pad: 'x'.repeat(80), i },
            timestamp: { millis: Date.now(), counter: i, nodeId: 'measure' },
          },
        });
      }
      client.messages.length = 0;
      client.send({ type: 'OP_BATCH', payload: { ops } });
      const lastId = ops[ops.length - 1].id;
      const deadline = Date.now() + BATCH_ACK_WAIT_MS;
      for (;;) {
        const answer = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], 1_000);
        if (answer?.type === 'OP_ACK' && answer.payload?.lastId === lastId) break;
        if (answer && answer.type !== 'OP_ACK') {
          throw new Error(
            `batch ending ${lastId} answered ${JSON.stringify(answer).slice(0, 400)}`,
          );
        }
        if (answer) client.messages.splice(client.messages.indexOf(answer), 1);
        if (Date.now() > deadline) throw new Error(`no OP_ACK for ${lastId}`);
      }
    }
  } finally {
    client.close();
  }
}

/** The leading timestamp of the last log line that contains `needle`, else of the last line. */
function lineTime(server: SpawnedBinary, needle?: string): string | null {
  const lines = plainLines(server.stdout()).filter((l) => l.trim() !== '');
  const line = needle
    ? [...lines].reverse().find((l) => l.includes(needle))
    : lines[lines.length - 1];
  return line ? (/^(\S+)/.exec(line.trim())?.[1] ?? null) : null;
}

/** Frames the start that wrote this log replayed: the base's per-partition `count`s, or the new `frames`. */
function replayedFrames(server: SpawnedBinary, arm: Arm): number | null {
  const lines = plainLines(server.stdout());
  if (arm === 'b') {
    return lines
      .filter((l) => l.includes('WAL recovery: replaying unapplied entries'))
      .reduce((sum, l) => sum + Number(logField(l, 'count') ?? 'NaN'), 0);
  }
  const done = lines.find((l) => l.includes('WAL recovery complete'));
  return done ? Number(logField(done, 'frames')) : null;
}

function exitedCleanly(exit: BinaryExit | null): boolean {
  return exit !== null && exit.code === 0;
}

async function oneRun(
  binaries: Binaries,
  position: number,
  arm: Arm,
  attempt: number,
): Promise<RunRecord> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-batch-measure-${position}${arm}-`));
  const record: RunRecord = { position, arm, attempt, dir, valid: false, checks: {} };
  const invalid = (reason: string) => {
    record.invalidReason = reason;
    return record;
  };
  const label = (what: string) => `${arm === 'b' ? 'BASE' : 'NEW'} #${position} ${what}`;

  // D and F.
  let { server } = await start(binaries, arm, dir, label('D'));
  await fill(server.port as number, 'd', N);
  if (arm === 'n') {
    const passes = await scrapeGauge(
      server.port as number,
      'topgun_write_behind_flush_passes_total',
    );
    record.checks.passesBeforeSignal = passes;
    if (passes !== 0) {
      await server.stop('SIGKILL');
      return invalid(`a flush pass ran before the signal (passes ${passes})`);
    }
  }
  record.checks.signalAt = new Date().toISOString();
  const t0 = process.hrtime.bigint();
  server.signal('SIGTERM');
  const exit = await server.waitForExit(DRAIN_EXIT_WAIT_MS);
  const t1 = process.hrtime.bigint();
  record.drainSeconds = Number(t1 - t0) / 1e9;
  record.D = N / record.drainSeconds;
  record.checks.drainExit = exit;
  record.checks.drainEndLogTime =
    arm === 'b' ? lineTime(server) : lineTime(server, 'rollback hints written');
  const log = server.stdout();
  if (arm === 'b') {
    record.checks.drainTimedOut = log.includes('Shutdown drain timed out');
    if (!exitedCleanly(exit) || record.checks.drainTimedOut) {
      if (!exit) await server.stop('SIGKILL');
      return invalid('the base drain did not finish cleanly');
    }
  } else {
    const hints = plainLines(log).find((l) => l.includes('rollback hints written'));
    record.checks.drained = hints ? logField(hints, 'drained') : null;
    if (!exitedCleanly(exit) || record.checks.drained !== 'true') {
      if (!exit) await server.stop('SIGKILL');
      return invalid('the drain did not finish with drained=true');
    }
  }
  record.F = fs.statSync(path.join(dir, 'topgun-data', 'engine.redb')).size;

  ({ server } = await start(binaries, arm, dir, label('after the drain')));
  record.checks.replayAfterDrain = replayedFrames(server, arm);
  if (arm === 'n') await fill(server.port as number, 'd', N);
  const afterDrainExit = await server.stop('SIGTERM', DRAIN_EXIT_WAIT_MS);
  record.checks.afterDrainExit = afterDrainExit;
  if (record.checks.replayAfterDrain !== 0 || !exitedCleanly(afterDrainExit)) {
    return invalid('the start after the drain replayed frames or did not stop cleanly');
  }
  if (arm === 'n') {
    const hints = plainLines(server.stdout()).find((l) => l.includes('rollback hints written'));
    record.checks.drained2 = hints ? logField(hints, 'drained') : null;
    if (record.checks.drained2 !== 'true') {
      return invalid('the second drain did not finish with drained=true');
    }
    record.F2 = fs.statSync(path.join(dir, 'topgun-data', 'engine.redb')).size;
  }

  // S.
  ({ server } = await start(binaries, arm, dir, label('S fill')));
  const sessionsBefore = sessionsOpened;
  await fill(server.port as number, 's', N);
  record.checks.sSessions = sessionsOpened - sessionsBefore;
  await server.stop('SIGKILL');
  const replay = await start(binaries, arm, dir, label('S replay'));
  record.S = replay.startSeconds;
  record.checks.replayedFrames = replayedFrames(replay.server, arm);
  record.checks.replayExit = await replay.server.stop('SIGTERM', DRAIN_EXIT_WAIT_MS);
  if (record.checks.sSessions !== 1 || record.checks.replayedFrames !== N + 1) {
    return invalid(
      `the S writes used ${record.checks.sSessions} sessions and the start replayed ` +
        `${record.checks.replayedFrames} frames, not 1 session and ${N + 1} frames`,
    );
  }

  // P, new arm only.
  if (arm === 'n') {
    const pDir = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-batch-measure-${position}p-`));
    record.checks.pDir = pDir;
    const p = await start(binaries, arm, pDir, label('P'), FLUSH_INTERVAL_P_MS);
    await fill(p.server.port as number, 'p', N);
    const deadline = Date.now() + P_WAIT_MS;
    let scraped: { buffered: number | null; passes: number | null; p99: number | null } = {
      buffered: null,
      passes: null,
      p99: null,
    };
    while (Date.now() < deadline) {
      const body = await (await fetch(`http://localhost:${p.server.port}/metrics`)).text();
      const series = (name: string): number | null => {
        const line = body.split('\n').find((l) => l.trim().split(/\s+/)[0] === name);
        return line ? Number(line.trim().split(/\s+/).pop()) : null;
      };
      scraped = {
        buffered: series('topgun_write_behind_buffered_entries'),
        passes: series('topgun_write_behind_flush_passes_total'),
        p99: series('topgun_write_behind_flush_pass_seconds{quantile="0.99"}'),
      };
      if (scraped.buffered === 0 && (scraped.passes ?? 0) >= 10) break;
      await sleep(200);
    }
    record.checks.pScrape = scraped;
    await p.server.stop('SIGTERM', DRAIN_EXIT_WAIT_MS);
    if (scraped.buffered !== 0 || (scraped.passes ?? 0) < 10 || scraped.p99 === null) {
      return invalid('no scrape with an empty buffer after at least 10 passes');
    }
    record.P99 = scraped.p99;
  }

  record.valid = true;
  return record;
}

async function series(binaries: Binaries, tag: string): Promise<RunRecord[]> {
  const runs: RunRecord[] = [];
  for (const [i, arm] of ORDER.entries()) {
    let run = await oneRun(binaries, i + 1, arm, 1);
    runs.push(run);
    if (!run.valid) {
      run = await oneRun(binaries, i + 1, arm, 2);
      runs.push(run);
      if (!run.valid) {
        throw new StopError(`${tag}: position ${i + 1} (${arm}) invalid twice`, runs);
      }
    }
    process.stderr.write(`[measure] ${tag} ${JSON.stringify(run)}\n`);
  }
  return runs.filter((r) => r.valid);
}

class StopError extends Error {
  constructor(
    message: string,
    readonly runs: RunRecord[],
  ) {
    super(message);
  }
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

function values(runs: RunRecord[], arm: Arm, metric: Metric | 'P99'): number[] {
  return runs.filter((r) => r.arm === arm).map((r) => r[metric] as number);
}

function spread(runs: RunRecord[], metric: Metric): number {
  const base = values(runs, 'b', metric);
  return Math.max(...base) / Math.min(...base);
}

interface Verdict {
  metric: string;
  judgedBy: string;
  value: number;
  bound: string;
  pass: boolean;
}

function byArmValues(runs: RunRecord[], metric: Metric): Verdict {
  const ratio = median(values(runs, 'n', metric)) / median(values(runs, 'b', metric));
  return verdict(metric, 'arm values (median of three runs)', ratio);
}

function byWorstPairing(runs: RunRecord[], metric: Metric): Verdict {
  const base = values(runs, 'b', metric);
  const next = values(runs, 'n', metric);
  const ratio =
    metric === 'D' ? Math.min(...next) / Math.max(...base) : Math.max(...next) / Math.min(...base);
  return verdict(metric, 'worst pairing of the re-run', ratio);
}

function verdict(metric: Metric, judgedBy: string, ratio: number): Verdict {
  switch (metric) {
    case 'D':
      return { metric, judgedBy, value: ratio, bound: `>= ${GATE.dMin}`, pass: ratio >= GATE.dMin };
    case 'S':
      return { metric, judgedBy, value: ratio, bound: `<= ${GATE.sMax}`, pass: ratio <= GATE.sMax };
    case 'F':
      return { metric, judgedBy, value: ratio, bound: `<= ${GATE.fMax}`, pass: ratio <= GATE.fMax };
  }
}

describe('write-behind batch measurement (base against new, out of process)', () => {
  test(
    'drain >= 20x, start replay <= 0.25x, file <= 2.25x of the base and not grown by a rewrite; pass p99 <= 1 s',
    async () => {
      const binaries = requireBinaries();
      const outFile =
        process.env.MEASURE_OUT_FILE ||
        path.join(os.tmpdir(), `topgun-batch-measure-${Date.now()}.json`);
      const result: Record<string, unknown> = {
        base: binaries.base,
        next: binaries.next,
        n: N,
        order: ORDER.join(','),
        noiseBound: NOISE_BOUND,
        gate: GATE,
        note:
          'k = 1 run value per metric: the drain consumes the buffer and ends the process, so a run ' +
          'cannot repeat it. S is timed from just before the launcher hashes and spawns the binary to ' +
          'PORT=, the same launcher on both arms.',
      };
      const write = () => fs.writeFileSync(outFile, JSON.stringify(result, null, 2));
      try {
        const first = await series(binaries, 'series 1');
        result.series1 = first;
        const spreads1 = Object.fromEntries(
          (['D', 'S', 'F'] as Metric[]).map((m) => [m, spread(first, m)]),
        );
        result.spreads1 = spreads1;
        const over = (['D', 'S', 'F'] as Metric[]).filter((m) => spreads1[m] > NOISE_BOUND);
        let rerun: RunRecord[] | null = null;
        if (over.length > 0) {
          rerun = await series(binaries, 'series 2');
          result.series2 = rerun;
          result.spreads2 = Object.fromEntries(
            (['D', 'S', 'F'] as Metric[]).map((m) => [m, spread(rerun as RunRecord[], m)]),
          );
        }
        const verdicts: Verdict[] = (['D', 'S', 'F'] as Metric[]).map((m) => {
          if (!over.includes(m)) return byArmValues(first, m);
          const again = rerun as RunRecord[];
          return spread(again, m) <= NOISE_BOUND ? byArmValues(again, m) : byWorstPairing(again, m);
        });
        const grown = [...first, ...(rerun ?? [])].filter(
          (r) => r.arm === 'n' && !((r.F2 as number) <= (r.F as number)),
        );
        verdicts.push({
          metric: 'F2',
          judgedBy: 'every new run of every series: F2 <= F',
          value: grown.length,
          bound: '0 runs where the file grew',
          pass: grown.length === 0,
        });
        const pRuns = rerun ?? first;
        const worstP99 = Math.max(...values(pRuns, 'n', 'P99'));
        verdicts.push({
          metric: 'P99',
          judgedBy: `worst of the new runs of ${rerun ? 'the re-run' : 'series 1'}`,
          value: worstP99,
          bound: `<= ${GATE.p99MaxS} s`,
          pass: worstP99 <= GATE.p99MaxS,
        });
        result.verdicts = verdicts;
        result.pass = verdicts.every((v) => v.pass);
      } catch (err) {
        result.error = (err as Error).message;
        if (err instanceof StopError) result.invalidRuns = err.runs;
        write();
        throw err;
      }
      write();
      process.stderr.write(`[measure] JSON: ${outFile}\n${JSON.stringify(result.verdicts)}\n`);
      expect(result.verdicts).toEqual(
        (result.verdicts as Verdict[]).map((v) => ({ ...v, pass: true })),
      );
    },
    SUITE_TIMEOUT_MS,
  );
});
