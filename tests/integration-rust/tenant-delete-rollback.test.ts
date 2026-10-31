/**
 * Integration: an older binary reads a store after a tenant delete.
 *
 * The newer binary runs with tenancy on: tenant `b` writes `b.x` and `b.y`,
 * tenant `c` writes `c.keep` and `c.two`. The operator deletes tenant `b`,
 * deletes its `tenant-deleted:b` policy (so a refusal by policy cannot pass for
 * an absent record) and stops the node cleanly (exit 0, `drained=true`). The
 * older binary (built at 727db547, before tenants) then starts on the same
 * data and WAL directories: it serves every record of `c` and no record of
 * `b`. A tenant delete adds no metadata name, key space or WAL frame kind, so
 * the older binary has nothing to refuse.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (TENANT_OLD_SERVER_BINARY / TENANT_OLD_SERVER_COMMIT for the older
 * one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles.
 */

import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';

import { createRustTestClient, createLWWRecord, TestClient } from './helpers';
import {
  spawnBinary,
  firstFrame,
  queryKey,
  observe,
  sleep,
  plainLines,
  SpawnedBinary,
  BinaryExit,
} from './helpers/two-binary';

const OLD_COMMIT_PREFIX = '727db547';

const BUILD_HINT =
  `Tenant delete rollback: build the server at ${OLD_COMMIT_PREFIX} (the last binary before tenants) ` +
  'in its own worktree and target directory (`git worktree add <scratch>/topgun-tenant-old ' +
  `${OLD_COMMIT_PREFIX}\`, then \`cargo build --release -p topgun-server --manifest-path ` +
  '<scratch>/topgun-tenant-old/Cargo.toml --target-dir <scratch>/topgun-tenant-old-target`), build ' +
  'the newer one in this checkout, and export TENANT_OLD_SERVER_BINARY, TENANT_OLD_SERVER_COMMIT (the ' +
  'full commit the older binary was built from), RUST_SERVER_BINARY and NEW_SERVER_COMMIT.';

interface Binaries {
  oldBinary: string;
  oldCommit: string;
  newBinary: string;
  newCommit: string;
}

function executable(name: string): string {
  const value = process.env[name] as string;
  try {
    if (!fs.statSync(value).isFile()) throw new Error('not a regular file');
    fs.accessSync(value, fs.constants.X_OK);
  } catch (err) {
    throw new Error(
      `${name} does not name an executable file (${value}: ${(err as Error).message}). ${BUILD_HINT}`,
    );
  }
  return value;
}

/** The four variables, or a failure naming every missing one; never a skip. */
function requireBinaries(): Binaries {
  const names = [
    'TENANT_OLD_SERVER_BINARY',
    'TENANT_OLD_SERVER_COMMIT',
    'RUST_SERVER_BINARY',
    'NEW_SERVER_COMMIT',
  ];
  const missing = names.filter((name) => !process.env[name]);
  if (missing.length > 0) {
    throw new Error(`${missing.join(', ')} not set. ${BUILD_HINT}`);
  }
  const oldCommit = process.env.TENANT_OLD_SERVER_COMMIT as string;
  if (!oldCommit.startsWith(OLD_COMMIT_PREFIX)) {
    throw new Error(
      `TENANT_OLD_SERVER_COMMIT is ${oldCommit}, not a commit starting ${OLD_COMMIT_PREFIX}. ${BUILD_HINT}`,
    );
  }
  return {
    oldBinary: executable('TENANT_OLD_SERVER_BINARY'),
    oldCommit,
    newBinary: executable('RUST_SERVER_BINARY'),
    newCommit: process.env.NEW_SERVER_COMMIT as string,
  };
}

const JWT_SECRET = 'test-e2e-secret';
const TENANT_CLAIM = 'org_id';
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, JWT_SECRET, {
  expiresIn: '1h',
});
const RECORDS: Record<string, [string, string][]> = {
  b: [
    ['b.x', 'k1'],
    ['b.y', 'k1'],
  ],
  c: [
    ['c.keep', 'k1'],
    ['c.two', 'k1'],
  ],
};
const ACK_WAIT_MS = 5_000;
const FLUSH_SETTLE_MS = 3_000;
const STOP_TIMEOUT_MS = 40_000;
const CASE_TIMEOUT_MS = 180_000;
const CLEAN_EXIT: BinaryExit = { code: 0, signal: null };

function storeEnv(dir: string, extra: Record<string, string> = {}): Record<string, string> {
  return {
    STORAGE_BACKEND: 'redb',
    TOPGUN_DATA_DIR: path.join(dir, 'topgun-data'),
    TOPGUN_REDB_PATH: path.join(dir, 'topgun.redb'),
    TOPGUN_WAL_DIR: path.join(dir, 'wal'),
    TOPGUN_WAL_FSYNC_POLICY: 'per_op',
    TOPGUN_INDEX_PATH: path.join(dir, 'scalar_indexes.json'),
    TOPGUN_VECTOR_INDEX_PATH: path.join(dir, 'vector_indexes.json'),
    ...extra,
  };
}

function valueOf(map: string, key: string): Record<string, string> {
  return { map, key };
}

async function admin(port: number, method: string, route: string, body?: unknown): Promise<any> {
  const resp = await fetch(`http://127.0.0.1:${port}${route}`, {
    method,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${ADMIN_TOKEN}` },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await resp.text();
  return { status: resp.status, body: text ? JSON.parse(text) : null };
}

async function tenantClient(port: number, tenant: string): Promise<TestClient> {
  const client = await createRustTestClient(port, { userId: `user-${tenant}`, autoAuth: false });
  await client.waitForMessage('AUTH_REQUIRED', ACK_WAIT_MS);
  const token = jwt.sign(
    { sub: `user-${tenant}`, roles: ['USER'], [TENANT_CLAIM]: tenant },
    JWT_SECRET,
    { expiresIn: '1h' },
  );
  client.send({ type: 'AUTH', token });
  await client.waitForMessage('AUTH_ACK', ACK_WAIT_MS);
  client.stopHeartbeat();
  return client;
}

async function writeAll(port: number): Promise<Record<string, string>> {
  const answers: Record<string, string> = {};
  for (const [tenant, records] of Object.entries(RECORDS)) {
    const client = await tenantClient(port, tenant);
    try {
      for (const [map, key] of records) {
        client.messages.length = 0;
        client.send({
          type: 'CLIENT_OP',
          payload: {
            id: `${map}-${key}`,
            mapName: map,
            opType: 'PUT',
            key,
            record: createLWWRecord(valueOf(map, key)),
          },
        });
        const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
        answers[`${map}/${key}`] = frame?.type ?? 'none';
      }
    } finally {
      client.close();
    }
  }
  return answers;
}

async function readAll(port: number): Promise<Record<string, unknown>> {
  const values: Record<string, unknown> = {};
  let i = 0;
  for (const records of Object.values(RECORDS)) {
    for (const [map, key] of records) {
      values[`${map}/${key}`] = await observe(() => queryKey(port, map, key, `old-q-${i++}`));
    }
  }
  return values;
}

interface Observed {
  writes: Record<string, string>;
  allow?: number;
  deleted?: { status: number; body: any };
  policyDeleted?: number;
  newStop?: BinaryExit | null;
  drained?: boolean;
  oldPort?: number | null;
  oldRefusal?: string[];
  values: Record<string, unknown>;
  oldStop?: BinaryExit | null;
}

describe('Integration: an older binary serves a store after a tenant delete', () => {
  const o: Observed = { writes: {}, values: {} };
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const binaries = requireBinaries();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-tenant-delete-rollback-'));

    const fresh = await spawnBinary({
      binaryPath: binaries.newBinary,
      commit: binaries.newCommit,
      label: 'NEW (tenancy on)',
      env: storeEnv(dir, { TOPGUN_TENANT_CLAIM: TENANT_CLAIM }),
    });
    running.push(fresh);
    if (fresh.port === null) throw new Error(`NEW did not start:\n${fresh.stderr()}`);
    o.allow = (
      await admin(fresh.port, 'POST', '/api/admin/policies', {
        id: 'allow-all',
        mapPattern: '**',
        action: 'all',
        effect: 'allow',
      })
    ).status;
    o.writes = await writeAll(fresh.port);
    await sleep(FLUSH_SETTLE_MS);
    o.deleted = await admin(fresh.port, 'DELETE', '/api/admin/tenants/b');
    o.policyDeleted = (
      await admin(fresh.port, 'DELETE', '/api/admin/policies/tenant-deleted:b')
    ).status;
    o.newStop = await fresh.stop('SIGTERM', STOP_TIMEOUT_MS);
    o.drained = plainLines(fresh.stdout()).some(
      (l) => l.includes('rollback hints written') && l.includes('drained=true'),
    );

    const old = await spawnBinary({
      binaryPath: binaries.oldBinary,
      commit: binaries.oldCommit,
      label: 'OLD',
      env: storeEnv(dir),
    });
    running.push(old);
    o.oldPort = old.port;
    if (old.port !== null) {
      o.values = await readAll(old.port);
      o.oldStop = await old.stop('SIGTERM', STOP_TIMEOUT_MS);
    } else {
      o.oldRefusal = plainLines(`${old.stdout()}\n${old.stderr()}`)
        .filter((l) => /ERROR|refus|Error/.test(l))
        .map((l) => l.slice(0, 400));
    }

    process.stderr.write(`[tenant-delete-rollback] observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test(`the older binary is the build from ${OLD_COMMIT_PREFIX}`, () => {
    expect(requireBinaries().oldCommit.startsWith(OLD_COMMIT_PREFIX)).toBe(true);
  });

  test('setup: every write acknowledged, tenant b deleted, its policy removed, a drained clean stop', () => {
    expect(o.allow).toBe(201);
    expect(Object.values(o.writes)).toEqual(['OP_ACK', 'OP_ACK', 'OP_ACK', 'OP_ACK']);
    expect(o.deleted?.status).toBe(200);
    expect(o.deleted?.body?.dropped).toEqual(['b.x', 'b.y']);
    expect(o.policyDeleted).toBe(204);
    expect(o.newStop).toEqual(CLEAN_EXIT);
    expect(o.drained).toBe(true);
  });

  test('the older binary starts on the same directories, serves every record of c and none of b', () => {
    expect(o.oldRefusal).toBeUndefined();
    expect(typeof o.oldPort).toBe('number');
    expect(o.values).toEqual({
      'b.x/k1': undefined,
      'b.y/k1': undefined,
      'c.keep/k1': valueOf('c.keep', 'k1'),
      'c.two/k1': valueOf('c.two', 'k1'),
    });
    expect(o.oldStop).toEqual(CLEAN_EXIT);
  });
});
