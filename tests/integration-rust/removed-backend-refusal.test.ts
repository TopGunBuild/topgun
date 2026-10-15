/**
 * Integration: a server asked for the removed Postgres backend refuses to start
 * before it creates its embedded store, so an operator who expects Postgres
 * never gets a server running on an empty store instead.
 *
 * The listener is bound before the backend is chosen, so "no port is bound" is
 * not observable; the readiness line (`PORT=`) is printed only after recovery,
 * which a refused start never reaches. The harness drops an inherited
 * DATABASE_URL, so each arm sees only the variable it sets itself.
 */

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { spawnBinary, SpawnedBinary } from './helpers/two-binary';

const PINNED = [
  'the Postgres backend has been removed',
  'There is no migration tool',
  'postgres-backend-last',
];
const EXIT_WAIT_MS = 10_000;

function binary(): string {
  const p = process.env.RUST_SERVER_BINARY;
  if (!p) throw new Error('RUST_SERVER_BINARY must point at a release build of topgun-server');
  return p;
}

interface Arm {
  exit: { code: number | null; signal: string | null } | null;
  substrings: boolean[];
  readyLine: boolean;
  redbExists: boolean;
  walExists: boolean;
  ready: number | null;
}

async function runArm(label: string, extra: Record<string, string>): Promise<Arm> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-removed-backend-'));
  const redb = path.join(dir, 'topgun.redb');
  const wal = path.join(dir, 'topgun-wal');
  let server: SpawnedBinary | null = null;
  try {
    server = await spawnBinary({
      binaryPath: binary(),
      label,
      portTimeoutMs: EXIT_WAIT_MS,
      env: {
        TOPGUN_NO_AUTH: '1',
        TOPGUN_REDB_PATH: redb,
        TOPGUN_WAL_DIR: wal,
        // spawnBinary defaults this to `null`; an undefined value is left out of
        // the child's environment, so the arm without a variable gets the real default.
        STORAGE_BACKEND: undefined,
        ...extra,
      } as unknown as Record<string, string>,
    });
    const exit = server.port === null ? await server.waitForExit(EXIT_WAIT_MS) : null;
    let ready: number | null = null;
    if (server.port !== null) {
      const deadline = Date.now() + EXIT_WAIT_MS;
      while (Date.now() < deadline && ready !== 200) {
        ready = await fetch(`http://127.0.0.1:${server.port}/health/ready`)
          .then((r) => r.status)
          .catch(() => null);
        if (ready !== 200) await new Promise((r) => setTimeout(r, 100));
      }
    }
    const stderr = server.stderr();
    return {
      exit,
      substrings: PINNED.map((s) => stderr.includes(s)),
      readyLine: /^PORT=/m.test(server.stdout()),
      redbExists: fs.existsSync(redb),
      walExists: fs.existsSync(wal),
      ready,
    };
  } finally {
    if (server && server.exit() === null) await server.stop('SIGTERM', EXIT_WAIT_MS);
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const REFUSED: Arm = {
  exit: { code: 1, signal: null },
  substrings: [true, true, true],
  readyLine: false,
  redbExists: false,
  walExists: false,
  ready: null,
};

describe('Integration: the removed Postgres backend is refused at start', () => {
  test.each([
    ['a: STORAGE_BACKEND=postgres', { STORAGE_BACKEND: 'postgres' }],
    ['b: DATABASE_URL set', { DATABASE_URL: 'placeholder-not-a-url' }],
    ['b2: DATABASE_URL empty', { DATABASE_URL: '' }],
  ])('%s: exit 1, the refusal text, no store created', async (label, extra) => {
    const arm = await runArm(label, extra);
    process.stderr.write(`[removed-backend] ${label}: ${JSON.stringify(arm)}\n`);
    expect(arm).toEqual(REFUSED);
  });

  test('c: control, no variable starts on the embedded store', async () => {
    const arm = await runArm('control', {});
    process.stderr.write(`[removed-backend] control: ${JSON.stringify(arm)}\n`);
    expect(arm.readyLine).toBe(true);
    expect(arm.ready).toBe(200);
    expect(arm.redbExists).toBe(true);
  });
});
