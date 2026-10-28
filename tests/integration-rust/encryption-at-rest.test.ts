/**
 * Integration: a store sealed under a master key opens only with that key.
 *
 * A node started with `TOPGUN_ENCRYPTION_AT_REST=on` and the `file` provider
 * seals every record value and keeps its data keys wrapped under the master
 * key in the data directory (TG-ENC-001). Three cases, all out of process:
 *
 * - TG-ENC-006 — a sealed node started with its key file missing,
 *   unreadable, accessible by others, malformed, or holding another key exits
 *   1 with `encryption: refusing to start:`, never prints `PORT=`, changes no
 *   file of the WAL directory, and leaves nothing answering on its port.
 * - TG-ENC-007 — a start with a new key and the old one as the
 *   previous key rotates the master key: afterwards the old key alone is
 *   refused and the new key alone serves every value.
 * - A backup taken before that rotation is refused with the new key
 *   alone, naming the fingerprint of the key it needs, and restores with the
 *   old key given as the previous key.
 *
 * Key files are written into a fresh directory under the OS temporary
 * directory and their contents are never printed.
 */

import * as crypto from 'crypto';
import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';

import {
  BinaryExit,
  connectWithin,
  firstFrame,
  observe,
  queryKey,
  spawnBinary,
  SpawnedBinary,
  treeSnapshot,
  TreeSnapshot,
} from './helpers/two-binary';

const MAP = 'encryption_items';
const KEYS = ['k0', 'k1', 'k2', 'k3', 'k4', 'k5'];
const ACK_WAIT_MS = 5_000;
/** The default flush interval is 1 s; this leaves every write stored. */
const FLUSH_SETTLE_MS = 3_000;
const REFUSAL_DEADLINE_MS = 30_000;
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 300_000;
const REFUSAL_LINE = 'encryption: refusing to start:';

const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

function binary(): string {
  const p = process.env.RUST_SERVER_BINARY;
  if (!p) throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
  return p;
}

/** A fresh 256-bit master key, as the 64 hex characters the `file` provider reads. */
function newKek(): string {
  return crypto.randomBytes(32).toString('hex');
}

/** The first 8 bytes of SHA-256 over `"topgun/kek-fp/1" ‖ KEK`, as the server prints it. */
function fingerprint(kekHex: string): string {
  return crypto
    .createHash('sha256')
    .update(Buffer.from('topgun/kek-fp/1'))
    .update(Buffer.from(kekHex, 'hex'))
    .digest()
    .subarray(0, 8)
    .toString('hex');
}

function writeKeyFile(dir: string, name: string, content: string, mode = 0o600): string {
  const file = path.join(dir, name);
  fs.writeFileSync(file, content);
  fs.chmodSync(file, mode);
  return file;
}

function storeEnv(dir: string, extra: Record<string, string>): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    ...extra,
  };
}

function sealedWith(keyFile: string, previous?: string): Record<string, string> {
  return {
    TOPGUN_ENCRYPTION_AT_REST: 'on',
    TOPGUN_MASTER_KEY_PROVIDER: 'file',
    TOPGUN_MASTER_KEY_FILE: keyFile,
    ...(previous ? { TOPGUN_MASTER_KEY_PREVIOUS_FILE: previous } : {}),
  };
}

function valueOf(key: string, run: string): Record<string, string> {
  return { key, run };
}

function expectedValues(run: string): Record<string, unknown> {
  return Object.fromEntries(KEYS.map((key) => [key, valueOf(key, run)]));
}

async function writeAll(port: number, run: string): Promise<Record<string, unknown>> {
  const answers: Record<string, unknown> = {};
  for (const [i, key] of KEYS.entries()) {
    answers[key] = await observe(async () => {
      const client = await connectWithin(port, `enc-${run}-writer-${i}`);
      try {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: String(i + 1),
            mapName: MAP,
            opType: 'PUT',
            key,
            record: {
              value: valueOf(key, run),
              timestamp: { millis: Date.now(), counter: 0, nodeId: 'encryption' },
            },
          },
        });
        const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
        return frame?.type ?? 'none';
      } finally {
        client.close();
      }
    });
  }
  return answers;
}

async function readAll(port: number, tag: string): Promise<Record<string, unknown>> {
  const values: Record<string, unknown> = {};
  for (const [i, key] of KEYS.entries()) {
    values[key] = await observe(() => queryKey(port, MAP, key, `enc-${tag}-q-${i}`));
  }
  return values;
}

/** A port nothing listens on at the moment of the call. */
function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as net.AddressInfo;
      server.close(() => resolve(port));
    });
  });
}

/** Whether a TCP connection to the port is accepted within a second. */
function accepts(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = net.connect({ port, host: '127.0.0.1' });
    const done = (ok: boolean) => {
      socket.destroy();
      resolve(ok);
    };
    socket.setTimeout(1_000, () => done(false));
    socket.once('connect', () => done(true));
    socket.once('error', () => done(false));
  });
}

/** Starts a node that must serve and reads every value from it, then stops it cleanly. */
async function serveAndRead(
  running: SpawnedBinary[],
  label: string,
  env: Record<string, string>,
): Promise<{ port: number | null; values: Record<string, unknown>; stop: BinaryExit | null }> {
  const server = await spawnBinary({ binaryPath: binary(), label, env });
  running.push(server);
  const values = server.port === null ? {} : await readAll(server.port, label);
  const stop = await server.stop('SIGTERM', STOP_TIMEOUT_MS);
  return { port: server.port, values, stop };
}

/** What a start that must be refused did. */
interface RefusedStart {
  exit: BinaryExit | null;
  printedPort: boolean;
  refusalLines: string[];
  acceptedAfter: boolean;
  walBefore: TreeSnapshot;
  walAfter: TreeSnapshot;
}

async function startExpectingRefusal(
  running: SpawnedBinary[],
  label: string,
  dir: string,
  env: Record<string, string>,
): Promise<RefusedStart> {
  const walDir = path.join(dir, 'wal');
  const walBefore = treeSnapshot(walDir);
  const port = await freePort();
  const server = await spawnBinary({
    binaryPath: binary(),
    label,
    env,
    port,
    portTimeoutMs: REFUSAL_DEADLINE_MS,
  });
  running.push(server);
  let exit = server.exit();
  if (exit === null && server.port === null) exit = await server.waitForExit(REFUSAL_DEADLINE_MS);
  const printedPort = server.port !== null || /^PORT=/m.test(server.stdout());
  if (exit === null || printedPort) {
    exit = null;
    await server.stop('SIGKILL', 5_000);
  }
  const output = `${server.stdout()}\n${server.stderr()}`.replace(/\u001b\[[0-9;]*m/g, '');
  return {
    exit,
    printedPort,
    refusalLines: output
      .split('\n')
      .filter((l) => l.includes(REFUSAL_LINE))
      .map((l) => l.slice(0, 600)),
    acceptedAfter: await accepts(port),
    walBefore,
    walAfter: treeSnapshot(walDir),
  };
}

function expectRefused(start: RefusedStart | undefined, says: string): void {
  expect(start?.printedPort).toBe(false);
  expect(start?.exit).toEqual({ code: 1, signal: null });
  expect(start?.refusalLines.some((l) => l.includes(says))).toBe(true);
  expect(start?.acceptedAfter).toBe(false);
  expect(start?.walBefore.paths.length).toBeGreaterThan(0);
  expect(start?.walAfter).toEqual(start?.walBefore);
}

function copyTree(from: string, to: string): void {
  fs.cpSync(from, to, { recursive: true });
}

describe('Integration: a sealed store opens only with its master key', () => {
  const o: {
    writes?: Record<string, unknown>;
    sealedStop?: BinaryExit | null;
    refusals: Record<string, RefusedStart>;
    rotated?: { port: number | null; values: Record<string, unknown>; stop: BinaryExit | null };
    oldAlone?: RefusedStart;
    newAlone?: { port: number | null; values: Record<string, unknown>; stop: BinaryExit | null };
    backupNewAlone?: RefusedStart;
    backupRestored?: {
      port: number | null;
      values: Record<string, unknown>;
      stop: BinaryExit | null;
    };
  } = { refusals: {} };
  let root: string | null = null;
  let oldKek = '';
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-encryption-at-rest-'));
    const keys = path.join(root, 'keys');
    fs.mkdirSync(keys, { mode: 0o700 });
    const dir = path.join(root, 'node');
    const backup = path.join(root, 'backup');
    oldKek = newKek();
    const newKekHex = newKek();
    const oldKey = writeKeyFile(keys, 'old.key', `${oldKek}\n`);
    const newKey = writeKeyFile(keys, 'new.key', `${newKekHex}\n`);

    // --- A sealed node writes every key and stops cleanly.
    const first = await spawnBinary({
      binaryPath: binary(),
      label: 'sealed first run',
      env: storeEnv(dir, sealedWith(oldKey)),
    });
    running.push(first);
    if (first.port === null) throw new Error(`sealed node did not start:\n${first.stderr()}`);
    o.writes = await writeAll(first.port, 'sealed');
    await new Promise((resolve) => setTimeout(resolve, FLUSH_SETTLE_MS));
    o.sealedStop = await first.stop('SIGTERM', STOP_TIMEOUT_MS);

    // --- Every way the key can be unavailable.
    const unreadable = writeKeyFile(keys, 'unreadable.key', `${oldKek}\n`, 0o000);
    const permissive = writeKeyFile(keys, 'permissive.key', `${oldKek}\n`, 0o644);
    const malformed = writeKeyFile(keys, 'malformed.key', `${oldKek.slice(0, 63)}\n`);
    const variants: Record<string, string> = {
      missing: path.join(keys, 'absent.key'),
      unreadable,
      permissive,
      malformed,
      other: newKey,
    };
    for (const [name, keyFile] of Object.entries(variants)) {
      o.refusals[name] = await startExpectingRefusal(
        running,
        `refused: ${name}`,
        dir,
        storeEnv(dir, sealedWith(keyFile)),
      );
    }

    // --- A backup taken with the node stopped, before the rotation.
    fs.mkdirSync(backup);
    copyTree(path.join(dir, 'topgun-data'), path.join(backup, 'topgun-data'));
    copyTree(path.join(dir, 'wal'), path.join(backup, 'wal'));

    // --- Rotate, then the old key alone and the new key alone.
    o.rotated = await serveAndRead(running, 'rotation', storeEnv(dir, sealedWith(newKey, oldKey)));
    o.oldAlone = await startExpectingRefusal(
      running,
      'old key alone',
      dir,
      storeEnv(dir, sealedWith(oldKey)),
    );
    o.newAlone = await serveAndRead(running, 'new key alone', storeEnv(dir, sealedWith(newKey)));

    // --- The backup needs the old key as the previous key.
    o.backupNewAlone = await startExpectingRefusal(
      running,
      'backup, new key alone',
      backup,
      storeEnv(backup, sealedWith(newKey)),
    );
    o.backupRestored = await serveAndRead(
      running,
      'backup, rotated on restore',
      storeEnv(backup, sealedWith(newKey, oldKey)),
    );

    // Straight to stderr: the suite silences console.log. No key content is in `o`.
    process.stderr.write(`[encryption-at-rest] observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (root) fs.rmSync(root, { recursive: true, force: true });
  });

  test('setup: the sealed node acknowledged every write and stopped cleanly', () => {
    expect(o.writes).toEqual(Object.fromEntries(KEYS.map((key) => [key, 'OP_ACK'])));
    expect(o.sealedStop).toEqual(CLEAN_EXIT);
  });

  test.each(['missing', 'unreadable', 'permissive', 'malformed'])(
    'a key file that is %s is refused with exit 1 and nothing written',
    (name) => {
      expectRefused(o.refusals[name], 'provider file:');
    },
  );

  test('another key is refused with exit 1, naming the fingerprint the store needs', () => {
    expectRefused(o.refusals.other, fingerprint(oldKek));
  });

  test('the rotation start serves every value', () => {
    expect(typeof o.rotated?.port).toBe('number');
    expect(o.rotated?.values).toEqual(expectedValues('sealed'));
    expect(o.rotated?.stop).toEqual(CLEAN_EXIT);
  });

  test('after the rotation the old key alone is refused', () => {
    expectRefused(o.oldAlone, REFUSAL_LINE);
  });

  test('after the rotation the new key alone serves every value', () => {
    expect(typeof o.newAlone?.port).toBe('number');
    expect(o.newAlone?.values).toEqual(expectedValues('sealed'));
    expect(o.newAlone?.stop).toEqual(CLEAN_EXIT);
  });

  test('a backup from before the rotation is refused with the new key alone, naming the old fingerprint', () => {
    expectRefused(o.backupNewAlone, fingerprint(oldKek));
  });

  test('the backup restores with the old key as the previous key', () => {
    expect(typeof o.backupRestored?.port).toBe('number');
    expect(o.backupRestored?.values).toEqual(expectedValues('sealed'));
    expect(o.backupRestored?.stop).toEqual(CLEAN_EXIT);
  });
});
