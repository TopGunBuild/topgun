/**
 * Integration: stored permission policies across the templating change
 * (TG-AUTHZ-003).
 *
 * The older binary is the build from f3a85ae9, the last one without policy
 * placeholders and `keyPattern`; the newer one is the binary under test.
 *
 * 1. `scoped`. The newer binary stores a key-scoped policy and stops drained.
 *    The older binary on the same directories exits 1 with
 *    `FATAL: policy recovery failed: … decode policy '<id>'` on stderr and
 *    never prints `PORT=`: it does not read the policy as a plain one.
 * 2. `plain`. The newer binary stores only plain policies; the older binary
 *    starts on the directories and enforces them.
 * 3. `legacy brace`. The older binary stores a plain policy whose `mapPattern`
 *    is `t.{auth.id}`. The newer binary exits 1 naming the policy; after the
 *    way out (the older binary deletes it) the newer binary starts.
 * 4. `meaning change`. The older binary stores a deny on `**` whose condition
 *    references `foo.y`, beside an allow on `*`, and admits a write. The newer
 *    binary starts, logs the meaning-change `WARN` naming the policy, and
 *    refuses the same write.
 *
 * Variables: POLICY_OLD_SERVER_BINARY / POLICY_OLD_SERVER_COMMIT for the older
 * binary, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer. With any of
 * them missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';

import { CLEAN_EXIT, drainedStop, storeEnv } from './helpers/engine-migration';
import { createLWWRecord } from './helpers';
import {
  spawnBinary,
  connectWithin,
  firstFrame,
  plainLines,
  SpawnedBinary,
  BinaryExit,
  TwoBinaryEnv,
} from './helpers/two-binary';

const OLD_COMMIT_PREFIX = 'f3a85ae9';
const CASE_TIMEOUT_MS = 300_000;
const REFUSAL_DEADLINE_MS = 30_000;
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, 'test-e2e-secret', {
  expiresIn: '1h',
});
const ENV = { TOPGUN_ADMIN_SUBJECTS: 'operator', TOPGUN_WAL_FSYNC_POLICY: 'per_op' };

function requirePolicyRollbackEnv(): TwoBinaryEnv {
  const hint =
    `Policy rollback: build the server at ${OLD_COMMIT_PREFIX} in its own worktree and target ` +
    'directory, build the newer one in this checkout, and export POLICY_OLD_SERVER_BINARY, ' +
    'POLICY_OLD_SERVER_COMMIT, RUST_SERVER_BINARY and NEW_SERVER_COMMIT.';
  const names = [
    'POLICY_OLD_SERVER_BINARY',
    'POLICY_OLD_SERVER_COMMIT',
    'RUST_SERVER_BINARY',
    'NEW_SERVER_COMMIT',
  ];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length > 0) throw new Error(`${missing.join(', ')} not set. ${hint}`);
  for (const name of ['POLICY_OLD_SERVER_BINARY', 'RUST_SERVER_BINARY']) {
    fs.accessSync(process.env[name] as string, fs.constants.X_OK);
  }
  return {
    oldBinary: process.env.POLICY_OLD_SERVER_BINARY as string,
    oldCommit: process.env.POLICY_OLD_SERVER_COMMIT as string,
    newBinary: process.env.RUST_SERVER_BINARY as string,
    newCommit: process.env.NEW_SERVER_COMMIT as string,
  };
}

type Role = 'old' | 'new';

async function start(
  running: SpawnedBinary[],
  env: TwoBinaryEnv,
  role: Role,
  dir: string,
  label: string,
): Promise<SpawnedBinary & { port: number }> {
  const server = await spawnBinary({
    binaryPath: role === 'old' ? env.oldBinary : env.newBinary,
    commit: role === 'old' ? env.oldCommit : env.newCommit,
    label,
    env: storeEnv(dir, 'rocksdb', ENV),
    portTimeoutMs: 120_000,
  });
  running.push(server);
  if (server.port === null) {
    throw new Error(`${label} did not start:\n${server.stdout().slice(-4000)}\n${server.stderr()}`);
  }
  return server as SpawnedBinary & { port: number };
}

interface Refusal {
  exit: BinaryExit | null;
  printedPort: boolean;
  fatalNamesPolicy: boolean;
}

/** Starts a binary that must refuse; kills it if it does not. */
async function refusedStart(
  running: SpawnedBinary[],
  env: TwoBinaryEnv,
  role: Role,
  dir: string,
  policyId: string,
): Promise<Refusal> {
  const server = await spawnBinary({
    binaryPath: role === 'old' ? env.oldBinary : env.newBinary,
    commit: role === 'old' ? env.oldCommit : env.newCommit,
    label: `${role.toUpperCase()} expected to refuse`,
    env: storeEnv(dir, 'rocksdb', ENV),
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
  const stderr = server.stderr();
  return {
    exit,
    printedPort,
    fatalNamesPolicy:
      stderr.includes('FATAL: policy recovery failed') &&
      stderr.includes(`decode policy '${policyId}'`),
  };
}

async function admin(port: number, method: string, url: string, body?: object): Promise<number> {
  const resp = await fetch(`http://127.0.0.1:${port}${url}`, {
    method,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${ADMIN_TOKEN}` },
    body: body ? JSON.stringify(body) : undefined,
  });
  await resp.text();
  return resp.status;
}

/** One OP_BATCH write by a client that is not an admin subject; the answer's frame type. */
async function write(port: number, map: string, key: string, value: unknown): Promise<string> {
  const client = await connectWithin(port, `writer-${map}-${key}`);
  try {
    client.messages.length = 0;
    client.send({
      type: 'OP_BATCH',
      payload: {
        ops: [
          {
            id: `op-${map}-${key}`,
            mapName: map,
            opType: 'PUT',
            key,
            record: createLWWRecord(value),
          },
        ],
      },
    });
    const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], 10_000);
    return frame?.type ?? 'none';
  } finally {
    client.close();
  }
}

test(`the older binary is the build from ${OLD_COMMIT_PREFIX}`, () => {
  expect(requirePolicyRollbackEnv().oldCommit.startsWith(OLD_COMMIT_PREFIX)).toBe(true);
});

describe('Integration: policy rollback across templating', () => {
  const o: Record<string, any> = {};
  const dirs: string[] = [];
  const running: SpawnedBinary[] = [];
  const mkdir = (tag: string) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), `topgun-policy-rollback-${tag}-`));
    dirs.push(dir);
    return dir;
  };

  beforeAll(async () => {
    const env = requirePolicyRollbackEnv();

    // 1. scoped: the newer binary stores a key-scoped policy.
    {
      const dir = mkdir('scoped');
      const fresh = await start(running, env, 'new', dir, 'NEW stores a key-scoped policy');
      o.scopedCreate = await admin(fresh.port, 'POST', '/api/admin/policies', {
        id: 'presence-own',
        mapPattern: 't.presence',
        keyPattern: '{auth.id}:*',
        action: 'write',
        effect: 'allow',
      });
      o.scopedStop = await drainedStop(fresh);
      o.scopedOld = await refusedStart(running, env, 'old', dir, 'presence-own');
    }

    // 2. plain: the older binary starts on plain policies and enforces them.
    {
      const dir = mkdir('plain');
      const fresh = await start(running, env, 'new', dir, 'NEW stores plain policies');
      o.plainCreate = await admin(fresh.port, 'POST', '/api/admin/policies', {
        id: 'docs-write',
        mapPattern: 'docs',
        action: 'write',
        effect: 'allow',
      });
      o.plainStop = await drainedStop(fresh);
      const old = await start(running, env, 'old', dir, 'OLD on plain policies');
      o.plainOldWrites = {
        docs: await write(old.port, 'docs', 'k', { v: 1 }),
        other: await write(old.port, 'other', 'k', { v: 1 }),
      };
      o.plainOldStop = await drainedStop(old);
    }

    // 3. legacy brace: the older binary stores `t.{auth.id}` as a literal.
    {
      const dir = mkdir('brace');
      const old = await start(running, env, 'old', dir, 'OLD stores a brace pattern');
      o.braceCreate = await admin(old.port, 'POST', '/api/admin/policies', {
        id: 'legacy-brace',
        mapPattern: 't.{auth.id}',
        action: 'read',
        effect: 'allow',
      });
      o.braceOldStop = await drainedStop(old);
      o.braceNew = await refusedStart(running, env, 'new', dir, 'legacy-brace');
      const wayOut = await start(running, env, 'old', dir, 'OLD deletes the policy');
      o.braceDelete = await admin(wayOut.port, 'DELETE', '/api/admin/policies/legacy-brace');
      o.braceWayOutStop = await drainedStop(wayOut);
      const again = await start(running, env, 'new', dir, 'NEW after the way out');
      o.braceNewStarts = again.port !== null;
      o.braceNewStop = await drainedStop(again);
    }

    // 4. meaning change: a stored unknown namespace now makes a deny apply.
    {
      const dir = mkdir('meaning');
      const old = await start(running, env, 'old', dir, 'OLD stores a foo.y deny');
      o.meaningCreate = [
        await admin(old.port, 'POST', '/api/admin/policies', {
          id: 'allow-all',
          mapPattern: '*',
          action: 'all',
          effect: 'allow',
        }),
        await admin(old.port, 'POST', '/api/admin/policies', {
          id: 'deny-foo',
          mapPattern: '**',
          action: 'write',
          effect: 'deny',
          conditionExpr: 'data.a == foo.y',
        }),
      ];
      o.meaningOldWrite = await write(old.port, 'docs', 'k1', { a: 1 });
      o.meaningOldStop = await drainedStop(old);
      const fresh = await start(running, env, 'new', dir, 'NEW reads the foo.y deny');
      o.meaningWarn = plainLines(fresh.stdout()).some(
        (l) =>
          l.includes('WARN') &&
          l.includes('policy deny-foo: foo.y is not a known condition namespace'),
      );
      o.meaningNewWrite = await write(fresh.port, 'docs', 'k2', { a: 1 });
      o.meaningNewStop = await drainedStop(fresh);
    }

    process.stderr.write(`[policy-rollback] observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS * 2);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    for (const dir of dirs) fs.rmSync(dir, { recursive: true, force: true });
  });

  const drained = { exit: CLEAN_EXIT, drained: true };

  test('1. the older binary refuses a store holding a key-scoped policy, naming it', () => {
    expect(o.scopedCreate).toBe(201);
    expect(o.scopedStop).toEqual(drained);
    expect(o.scopedOld).toEqual({
      exit: { code: 1, signal: null },
      printedPort: false,
      fatalNamesPolicy: true,
    });
  });

  test('2. on plain policies the older binary starts and enforces them', () => {
    expect(o.plainCreate).toBe(201);
    expect(o.plainStop).toEqual(drained);
    expect(o.plainOldWrites).toEqual({ docs: 'OP_ACK', other: 'OP_REJECTED' });
    expect(o.plainOldStop).toEqual(drained);
  });

  test('3. a stored brace pattern stops the newer binary until the older one deletes it', () => {
    expect(o.braceCreate).toBe(201);
    expect(o.braceOldStop).toEqual(drained);
    expect(o.braceNew).toEqual({
      exit: { code: 1, signal: null },
      printedPort: false,
      fatalNamesPolicy: true,
    });
    expect(o.braceDelete).toBe(204);
    expect(o.braceWayOutStop).toEqual(drained);
    expect(o.braceNewStarts).toBe(true);
    expect(o.braceNewStop).toEqual(drained);
  });

  test('4. a stored unknown-namespace deny now applies, with a startup warning', () => {
    expect(o.meaningCreate).toEqual([201, 201]);
    expect(o.meaningOldWrite).toBe('OP_ACK');
    expect(o.meaningOldStop).toEqual(drained);
    expect(o.meaningWarn).toBe(true);
    expect(o.meaningNewWrite).toBe('OP_REJECTED');
    expect(o.meaningNewStop).toEqual(drained);
  });
});
