/**
 * Integration: a write the store refuses must not stop the server.
 *
 * A write is acknowledged once it is in the write-ahead log; it reaches the
 * store later, in the background. This test makes the store refuse that later
 * step and then checks what an operator would care about: the server still
 * answers reads and new connections, it reports the write it gave up on, and
 * it exits on SIGTERM.
 *
 * The refusal comes from a server built with the `fault-injection` cargo
 * feature: while a sentinel file exists, every store write is refused. No
 * release build has that feature, so the binary is given by path
 * (FAULT_SERVER_BINARY) together with the commit it was built from
 * (FAULT_SERVER_COMMIT). With either missing the test fails; it never skips
 * and never falls back to another binary.
 *
 * Three arms, one server process each:
 *
 * - PINNED   — the stall watchdog ticks every 600 ms, so it meets every retry
 *              backoff of the refused write;
 * - DEFAULTS — the shipped configuration (the watchdog ticks every 6 s);
 * - CONTROL  — the PINNED configuration with the sentinel never created, so
 *              nothing is refused. It shows the probes pass on a healthy node.
 *
 * Environment:
 *
 * - LIVENESS_ARMS    — comma-separated arms to run (default: all three);
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

type Arm = 'PINNED' | 'DEFAULTS' | 'CONTROL';
const ALL_ARMS: Arm[] = ['CONTROL', 'PINNED', 'DEFAULTS'];

const BUILD_COMMAND =
  'cargo build --release -p topgun-server --features fault-injection --target-dir target/fault-injection';
const SAMPLE_TOOL = '/usr/bin/sample';

const CONTROL_MAP = 'liveness_control';
const CONTROL_KEY = 'alice';
const CONTROL_VALUE = { name: 'Alice', written: 'while the store was healthy' };
const REFUSED_MAP = 'liveness_refused';
const REFUSED_KEY = 'bob';
const REFUSED_VALUE = { name: 'Bob', written: 'while the store refuses' };

const SEAM_BOOT_TEXT = 'fault-injection build';
const DISCARD_TEXT = 'Write-behind entry discarded after max retries';

const ACK_WAIT_MS = 5_000;
const PROBE_WAIT_MS = 5_000;
/** Write delay (1 s) plus one flush tick (at most 1 s) plus slack: the control write is in the store by then. */
const CONTROL_FLUSH_WAIT_MS = 3_000;
/** Three attempts with 1 s and 2 s backoffs finish about 7 s after the ack; the window leaves room. */
const WINDOW_MS = 15_000;
const SIGTERM_EXIT_MS = 15_000;
const SIGTERM_OBSERVE_MS = 10_000;
const CLIENT_CLOSE_MS = 2_000;
const TEST_TIMEOUT_MS = 120_000;

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
  const hint = `Build the server with the store-refusal seam (\`${BUILD_COMMAND}\`) and export FAULT_SERVER_BINARY (its path) and FAULT_SERVER_COMMIT (the commit it was built from).`;
  const binary = process.env.FAULT_SERVER_BINARY;
  if (!binary) throw new Error(`FAULT_SERVER_BINARY is not set. ${hint}`);
  const commit = process.env.FAULT_SERVER_COMMIT;
  if (!commit) throw new Error(`FAULT_SERVER_COMMIT is not set. ${hint}`);
  try {
    if (!fs.statSync(binary).isFile()) throw new Error('not a regular file');
    fs.accessSync(binary, fs.constants.X_OK);
  } catch (err) {
    throw new Error(
      `FAULT_SERVER_BINARY does not name an executable file (${binary}: ${(err as Error).message}). ${hint}`,
    );
  }

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

function lwwPut(id: string, mapName: string, key: string, value: unknown): Record<string, unknown> {
  return {
    id,
    mapName,
    opType: 'PUT',
    key,
    record: { value, timestamp: { millis: Date.now(), counter: 0, nodeId: 'liveness' } },
  };
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

interface Observed {
  arm: Arm;
  /** `ok` when no discard line names the control map. */
  l0: 'ok' | 'fail';
  /** The control value as read back, or the reason it could not be read. */
  l1: unknown;
  /** `ok` when a new connection completed its handshake, otherwise the reason. */
  l2: string;
  /** Number of discard lines for the refused write. */
  l3: number;
  runningBeforeSigterm: boolean;
  s0: number;
  s10: number;
  x10: 'running' | 'exited';
  l4: BinaryExit | null;
  verdict: 'HUNG' | 'HEALTHY' | 'OTHER';
}

function isEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

async function runArm(arm: Arm): Promise<Observed> {
  const env = requireFaultEnv();
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-store-refusal-'));
  const sentinel = path.join(dataDir, 'refuse');
  const startedAt = Date.now();
  const notes: string[] = [];
  const note = (text: string) => {
    notes.push(`${new Date().toISOString()} +${Date.now() - startedAt}ms ${text}`);
  };
  const clients: TestClient[] = [];
  let server: SpawnedBinary | null = null;
  let summary = `store-refusal-liveness arm=${arm} commit=${env.commit} aborted=before the server was launched`;

  try {
    const serverEnv: Record<string, string> = {
      STORAGE_BACKEND: 'redb',
      TOPGUN_REDB_PATH: path.join(dataDir, 'topgun.redb'),
      TOPGUN_WAL_DIR: path.join(dataDir, 'wal'),
      TOPGUN_WAL_FSYNC_POLICY: 'per_op',
      TOPGUN_FAULT_STORE_REFUSE_FILE: sentinel,
    };
    if (arm !== 'DEFAULTS') serverEnv.TOPGUN_WAL_WATERMARK_STALL_BOUND_MS = '6000';

    // Step 1: boot, and make sure this is a binary that has the seam at all.
    server = await spawnBinary({
      binaryPath: env.binary,
      commit: env.commit,
      label: `store-refusal ${arm}`,
      env: serverEnv,
    });
    const launched = server;
    const abort = (reason: string): never => {
      summary = `store-refusal-liveness arm=${arm} sha256=${launched.sha256} commit=${env.commit} aborted=${reason}`;
      throw new Error(`${reason}\n--- stderr ---\n${launched.stderr()}`);
    };
    summary = `store-refusal-liveness arm=${arm} sha256=${launched.sha256} commit=${env.commit} aborted=the run did not reach its probes`;
    if (launched.port === null) abort('the server did not print PORT=');
    const port = launched.port as number;
    note(`PORT=${port} pid=${launched.process.pid}`);

    const seamDeadline = Date.now() + 2_000;
    const seamLine = () =>
      plainLines(launched.stdout()).find((line) => line.includes(SEAM_BOOT_TEXT));
    while (!seamLine() && Date.now() < seamDeadline) await sleep(25);
    if (!seamLine()) {
      abort(
        `the launched binary did not print its "${SEAM_BOOT_TEXT}" line: it has no store-refusal seam, so nothing it does is about a refused write`,
      );
    }
    note(`seam boot line: ${(seamLine() as string).slice(0, 300)}`);

    // Step 2: a write that reaches the store while it is healthy.
    const writer = await connectWithin(port, 'liveness-writer', PROBE_WAIT_MS);
    clients.push(writer);
    const controlAck = await writeOn(writer, lwwPut('1', CONTROL_MAP, CONTROL_KEY, CONTROL_VALUE));
    note(`control write answered with ${controlAck}`);
    if (controlAck !== 'OP_ACK')
      abort(`the control write was answered with ${controlAck}, not OP_ACK`);
    await sleep(CONTROL_FLUSH_WAIT_MS);

    // Step 3: from here on the store refuses every write.
    if (arm !== 'CONTROL') {
      fs.writeFileSync(sentinel, '');
      note('sentinel created: store writes are refused');
    } else {
      note('CONTROL: sentinel not created');
    }

    // Step 4: the write the store will refuse. It is acknowledged all the same: the WAL has it.
    const refusedAck = await writeOn(writer, lwwPut('2', REFUSED_MAP, REFUSED_KEY, REFUSED_VALUE));
    const ackedAt = Date.now();
    note(`refused-map write answered with ${refusedAck}`);
    if (refusedAck !== 'OP_ACK') {
      abort(
        `the write to ${REFUSED_MAP} was answered with ${refusedAck}, not OP_ACK within ${ACK_WAIT_MS} ms: the seam disturbs the acknowledgement path`,
      );
    }

    // Step 5: let the retries run their course.
    await sleep(Math.max(0, ackedAt + WINDOW_MS - Date.now()));
    note('end of the window');

    // Step 6: probes, collected first and asserted together by the caller.
    const controlDiscards = plainLines(launched.stdout()).filter(
      (line) => line.includes(DISCARD_TEXT) && line.includes(`map=${CONTROL_MAP}`),
    );
    const l0: 'ok' | 'fail' = controlDiscards.length === 0 ? 'ok' : 'fail';
    const l1 = await observe(() =>
      queryKey(port, CONTROL_MAP, CONTROL_KEY, 'liveness-read', PROBE_WAIT_MS),
    );
    note(`L1 ${JSON.stringify(l1)}`);
    const l2Client = await observe(() => connectWithin(port, 'liveness-connect', PROBE_WAIT_MS));
    let l2: string;
    if (typeof l2Client === 'string') {
      l2 = l2Client;
    } else {
      clients.push(l2Client);
      l2 = 'ok';
    }
    note(`L2 ${l2}`);
    const l3 = plainLines(launched.stdout()).filter(
      (line) =>
        line.includes(DISCARD_TEXT) &&
        line.includes(`map=${REFUSED_MAP}`) &&
        line.includes(`key=${REFUSED_KEY}`) &&
        line.includes('retries=3'),
    ).length;
    note(`L0 ${l0} L3 ${l3}`);

    // Step 7: the test's own connections go first, so the shutdown below is
    // not measured while it waits for them. The read probe closed its own
    // connection when it returned; the pause lets that close land as well.
    for (const client of clients.splice(0))
      note(`client ${client.nodeId}: ${await closeAndWait(client)}`);
    await sleep(500);

    // Step 8: where every thread of the server is, before any signal reaches it.
    let sampleName = 'off';
    if (env.sample) {
      sampleName = `${arm}-${env.run}.sample.txt`;
      const taken = await takeSample(
        launched.process.pid as number,
        path.join(env.outDir as string, sampleName),
      );
      note(`stack sample ${sampleName}: ${taken}`);
      if (taken !== 'taken')
        abort(`LIVENESS_SAMPLE=1 and the stack sample was not taken (${taken})`);
    }

    // Step 9: SIGTERM, watching whether the server writes anything or exits in the ten seconds after it.
    const runningBeforeSigterm = launched.exit() === null;
    const s0 = Buffer.byteLength(launched.stdout());
    note(`SIGTERM (running=${runningBeforeSigterm}, stdout ${s0} bytes)`);
    const stopping = launched.stop('SIGTERM', SIGTERM_EXIT_MS);
    await sleep(SIGTERM_OBSERVE_MS);
    const s10 = Buffer.byteLength(launched.stdout());
    const x10 = launched.exit() === null ? 'running' : 'exited';
    const l4 = await stopping;
    note(`after SIGTERM: stdout ${s10} bytes at +10 s, ${x10}; exit ${JSON.stringify(l4)}`);

    const l1Ok = isEqual(l1, CONTROL_VALUE);
    const l4Clean = isEqual(l4, { code: 0, signal: null });
    let verdict: Observed['verdict'] = 'OTHER';
    if (!l1Ok && l2 !== 'ok' && l4?.signal === 'SIGKILL') verdict = 'HUNG';
    else if (l0 === 'ok' && l1Ok && l2 === 'ok' && l4Clean) verdict = 'HEALTHY';

    // A server that has stopped writing prints no discard line whatever happened to the control write.
    const l0Shown = verdict === 'HUNG' ? 'n/a' : l0;
    summary =
      `store-refusal-liveness arm=${arm} sha256=${launched.sha256} commit=${env.commit} ` +
      `L0=${l0Shown} L1=${l1Ok ? 'ok' : 'fail'} L2=${l2 === 'ok' ? 'ok' : 'fail'} L3=${l3} ` +
      `S0=${s0} S10=${s10} X10=${x10} L4=${JSON.stringify(l4)} sample=${sampleName} verdict=${verdict}`;

    return { arm, l0, l1, l2, l3, runningBeforeSigterm, s0, s10, x10, l4, verdict };
  } finally {
    for (const client of clients) client.close();
    if (server) await server.stop('SIGKILL', 5_000);
    // Straight to stderr: the suite silences console.log, and this line is the record of the run.
    process.stderr.write(`${summary}\n`);
    if (env.outDir) {
      const base = path.join(env.outDir, `${arm}-${env.run}`);
      fs.writeFileSync(`${base}.notes.txt`, `${notes.join('\n')}\n${summary}\n`);
      fs.writeFileSync(`${base}.stdout.txt`, server ? server.stdout() : '');
      fs.writeFileSync(`${base}.stderr.txt`, server ? server.stderr() : '');
    }
    fs.rmSync(dataDir, { recursive: true, force: true });
  }
}

describe('Integration: the server keeps serving after the store refuses an acknowledged write', () => {
  for (const arm of selectedArms()) {
    const title =
      arm === 'CONTROL'
        ? 'CONTROL: with nothing refused, every probe passes and no write is discarded'
        : `${arm}: reads, new connections, the discard report and SIGTERM all work after a refused write`;
    test(
      title,
      async () => {
        const o = await runArm(arm);
        expect({
          controlWriteDiscarded: o.l0 === 'fail',
          controlRead: o.l1,
          newConnection: o.l2,
          discardLinesForRefusedWrite: o.l3,
          runningBeforeSigterm: o.runningBeforeSigterm,
          exitOnSigterm: o.l4,
          verdict: o.verdict,
        }).toEqual({
          controlWriteDiscarded: false,
          controlRead: CONTROL_VALUE,
          newConnection: 'ok',
          discardLinesForRefusedWrite: arm === 'CONTROL' ? 0 : 1,
          runningBeforeSigterm: true,
          exitOnSigterm: { code: 0, signal: null },
          verdict: 'HEALTHY',
        });
      },
      TEST_TIMEOUT_MS,
    );
  }
});
