/**
 * Integration: the newer binary migrates a store the older binary wrote in
 * `engine.redb` into `engine.rocksdb/` and serves every record as it was.
 *
 * Two stores, each written by the older binary (built at 7692c380, the last
 * binary that stores in `engine.redb`) and stopped with exit 0 and
 * `drained=true`, then started with the newer binary:
 *
 * - Sealed store (encryption at rest on): LWW records, an OR-Map key with two
 *   live values, and an LWW record with a TTL long enough to outlive the
 *   migration. The newer binary migrates once (`store engine: migration
 *   complete`, the boot line reads `store_engine=rocksdb`, `engine.redb` is
 *   kept as `engine.redb.pre-rocksdb`), serves every record with the stamp and
 *   TTL it was first stored with, holds no value in plaintext, and a second
 *   start migrates nothing. Once the server clock passes the TTL record's
 *   `timestamp.millis + ttlMs` it is hidden from reads and the expiry sweep
 *   deletes it.
 * - Tenant store (`TOPGUN_TENANT_CLAIM` set, one `**` allow policy): tenants
 *   `b` and `c` write, the operator deletes `b`
 *   (`DELETE /api/admin/tenants/b`). After the migration the newer binary
 *   serves `c`'s maps, lists nothing of `b`, and still has the
 *   `tenant-deleted:b` deny policy in force.
 *
 * The two stores are separate because a tenant-scoped client cannot use the
 * admin readers the sealed store is checked with.
 *
 * Two binaries are needed, each given by path together with the commit it was
 * built from (ENGINE_OLD_SERVER_BINARY / ENGINE_OLD_SERVER_COMMIT for the older
 * one, RUST_SERVER_BINARY / NEW_SERVER_COMMIT for the newer). With any of them
 * missing the test fails; it never runs one binary in both roles.
 */

import * as crypto from 'crypto';
import * as fs from 'fs';
import * as jwt from 'jsonwebtoken';
import * as os from 'os';
import * as path from 'path';

import { createRustTestClient, TestClient } from './helpers';
import {
  CLEAN_EXIT,
  COMPLETE_LINE,
  MIGRATING_LINE,
  drainedStop,
  kindOf,
  layout,
  linesWith,
  putLww,
  storeEnv,
} from './helpers/engine-migration';
import {
  requireEngineEnv,
  spawnBinary,
  connectWithin,
  firstFrame,
  queryKey,
  orMapValues,
  merkleLeaves,
  countBytesUnder,
  logField,
  observe,
  plainLines,
  sleep,
  SpawnedBinary,
  ENGINE_OLD_COMMIT_PREFIX,
} from './helpers/two-binary';

const LWW = 'engine_upgrade_lww';
const OR = 'engine_upgrade_or';
const TTL = 'engine_upgrade_ttl';
/** Present in every written value and nowhere else, so a plaintext copy on disk is found by a byte search. */
const PLAIN = 'engine-upgrade-plaintext';
const LWW_ENTRIES: [string, unknown][] = ['k0', 'k1', 'k2'].map((k) => [
  k,
  { k, text: `${PLAIN}-${k}` },
]);
const OR_VALUES = [`${PLAIN}-or-a`, `${PLAIN}-or-b`];
/** Outlives the older binary's run and the migrating start, and expires in the second start. */
const TTL_MS = 40_000;
const SWEEP_INTERVAL_MS = 1_000;
/** One sweep interval plus one cycle past expiry, doubled. */
const SWEEP_CEILING_MS = 4 * SWEEP_INTERVAL_MS;
const FLUSH_SETTLE_MS = 3_000;
const ACK_WAIT_MS = 10_000;
const CASE_TIMEOUT_MS = 240_000;

function keyEnv(dir: string): Record<string, string> {
  return {
    TOPGUN_ENCRYPTION_AT_REST: 'on',
    TOPGUN_MASTER_KEY_PROVIDER: 'file',
    TOPGUN_MASTER_KEY_FILE: path.join(dir, 'master.key'),
    TOPGUN_EXPIRY_SWEEP_INTERVAL_MS: String(SWEEP_INTERVAL_MS),
  };
}

async function orAdd(port: number): Promise<string[]> {
  const client = await connectWithin(port, 'engine-upgrade-or-writer');
  const answers: string[] = [];
  try {
    for (const [i, value] of OR_VALUES.entries()) {
      client.messages.length = 0;
      client.send({
        type: 'CLIENT_OP',
        payload: {
          id: `engine-upgrade-or-${i}`,
          mapName: OR,
          opType: 'OR_ADD',
          key: 'k',
          orRecord: {
            value,
            timestamp: { millis: Date.now(), counter: 0, nodeId: 'engine-upgrade' },
            tag: `engine-upgrade-or-${i}`,
          },
        },
      });
      const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
      answers.push(frame?.type ?? 'none');
    }
  } finally {
    client.close();
  }
  return answers;
}

/** What the server holds, read from its Merkle leaves: values, stamps, tags and TTLs. */
async function held(port: number, withTtl: boolean): Promise<Record<string, unknown>> {
  const out: Record<string, unknown> = {
    lww: Object.fromEntries(await merkleLeaves(port, LWW, false)),
    or: Object.fromEntries(await merkleLeaves(port, OR, true)),
  };
  if (withTtl) out.ttl = Object.fromEntries(await merkleLeaves(port, TTL, false));
  return out;
}

/** What a client reads: LWW values by query, the OR key's live values, the TTL record by query. */
async function served(port: number, tag: string): Promise<Record<string, unknown>> {
  const lww: Record<string, unknown> = {};
  for (const [key] of LWW_ENTRIES) {
    lww[key] = await observe(() => queryKey(port, LWW, key, `${tag}-q-${key}`));
  }
  const or = await observe(() => orMapValues(port, OR, 'k', `${tag}-or`));
  return {
    lww,
    or: Array.isArray(or) ? [...or].sort() : or,
    ttl: await observe(() => queryKey(port, TTL, 't', `${tag}-ttl`)),
  };
}

async function metric(port: number, series: string): Promise<number> {
  const text = await (await fetch(`http://127.0.0.1:${port}/metrics`)).text();
  const line = text.split('\n').find((l) => l.startsWith(`${series} `));
  return line ? Number(line.slice(series.length + 1)) : 0;
}

/** The `store_engine` field of the boot summary line, if any line carries one. */
function storeEngine(server: SpawnedBinary): string | undefined {
  for (const line of plainLines(server.stdout())) {
    const value = logField(line, 'store_engine');
    if (value !== undefined) return value;
  }
  return undefined;
}

describe('Integration: a sealed store with a TTL record migrates to RocksDB', () => {
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];

  beforeAll(async () => {
    const env = requireEngineEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-engine-upgrade-sealed-'));
    fs.writeFileSync(path.join(dir, 'master.key'), `${crypto.randomBytes(32).toString('hex')}\n`, {
      mode: 0o600,
    });
    const l = layout(dir);

    const old = await spawnBinary({
      binaryPath: env.oldBinary,
      commit: env.oldCommit,
      label: 'OLD sealed',
      env: storeEnv(dir, 'redb', keyEnv(dir)),
    });
    running.push(old);
    if (old.port === null) throw new Error(`OLD did not start:\n${old.stderr()}`);
    o.writtenAt = Date.now();
    o.ttlWrite = await putLww(old.port, TTL, [['t', `${PLAIN}-ttl`]], 'engine-upgrade-ttl', {
      ttlMs: TTL_MS,
    });
    o.lwwWrites = await putLww(old.port, LWW, LWW_ENTRIES, 'engine-upgrade-lww');
    o.orWrites = await orAdd(old.port);
    o.oldHeld = await held(old.port, true);
    o.oldServed = await served(old.port, 'old');
    await sleep(FLUSH_SETTLE_MS);
    o.oldStop = await drainedStop(old);
    o.beforeMigration = { redb: kindOf(l.redb), rocks: kindOf(l.rocks) };
    o.plainInOldStore = countBytesUnder(l.dataDir, PLAIN);

    const first = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'NEW first start (migrates)',
      env: storeEnv(dir, 'rocksdb', keyEnv(dir)),
      portTimeoutMs: 120_000,
    });
    running.push(first);
    if (first.port === null) throw new Error(`NEW did not start:\n${first.stderr()}`);
    o.firstMigrating = linesWith(first, MIGRATING_LINE).length;
    o.firstComplete = linesWith(first, COMPLETE_LINE).length;
    o.firstEngine = storeEngine(first);
    o.firstHeld = await held(first.port, true);
    o.firstHeldAt = Date.now();
    o.firstServed = await served(first.port, 'first');
    o.firstStop = await drainedStop(first);
    o.afterMigration = { redb: kindOf(l.redb), rocks: kindOf(l.rocks), kept: kindOf(l.kept) };
    o.plainInRocks = countBytesUnder(l.rocks, PLAIN);

    const second = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'NEW second start',
      env: storeEnv(dir, 'rocksdb', keyEnv(dir)),
    });
    running.push(second);
    if (second.port === null) throw new Error(`NEW did not start again:\n${second.stderr()}`);
    o.secondMigrating = linesWith(second, MIGRATING_LINE).length;
    o.secondComplete = linesWith(second, COMPLETE_LINE).length;
    o.secondEngine = storeEngine(second);
    o.secondHeld = await held(second.port, false);

    // Polled, never slept through: the record is deleted once the TTL counted
    // from its first write has run out.
    const deadline = o.writtenAt + TTL_MS + SWEEP_CEILING_MS;
    for (;;) {
      o.removed = await metric(second.port, 'topgun_expiry_removed_total{kind="lww_record"}');
      if (o.removed >= 1 || Date.now() >= deadline) break;
      await sleep(250);
    }
    o.expiredRead = await observe(() => queryKey(second.port as number, TTL, 't', 'expired-q'));
    o.expiredLeaves = Object.fromEntries(await merkleLeaves(second.port, TTL, false));
    o.secondStop = await drainedStop(second);

    process.stderr.write(`[engine-upgrade] sealed, observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test(`the older binary is the build from ${ENGINE_OLD_COMMIT_PREFIX}`, () => {
    expect(requireEngineEnv().oldCommit.startsWith(ENGINE_OLD_COMMIT_PREFIX)).toBe(true);
  });

  test('setup: the older binary acked every write, stored it sealed in engine.redb and stopped drained', () => {
    expect(o.ttlWrite).toEqual({ t: 'OP_ACK' });
    expect(o.lwwWrites).toEqual({ k0: 'OP_ACK', k1: 'OP_ACK', k2: 'OP_ACK' });
    expect(o.orWrites).toEqual(['OP_ACK', 'OP_ACK']);
    expect(o.oldServed).toEqual({
      lww: Object.fromEntries(LWW_ENTRIES),
      or: OR_VALUES,
      ttl: `${PLAIN}-ttl`,
    });
    expect(o.oldStop).toEqual({ exit: CLEAN_EXIT, drained: true });
    expect(o.beforeMigration).toEqual({ redb: 'file', rocks: 'absent' });
    expect(o.plainInOldStore).toBe(0);
  });

  test('the first start migrates once, boots on rocksdb and keeps engine.redb.pre-rocksdb', () => {
    expect(o.firstMigrating).toBe(1);
    expect(o.firstComplete).toBe(1);
    expect(o.firstEngine).toBe('rocksdb');
    expect(o.afterMigration).toEqual({ redb: 'absent', rocks: 'directory', kept: 'file' });
    expect(o.firstStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test('the migrated store holds every record with its first stamp, tag and TTL, and no plaintext', () => {
    // Read before expiry, or the TTL record would be compared after it was deleted.
    expect(o.firstHeldAt).toBeLessThan(o.writtenAt + TTL_MS);
    expect(o.firstHeld).toEqual(o.oldHeld);
    expect(o.firstServed).toEqual(o.oldServed);
    expect(o.plainInRocks).toBe(0);
  });

  test('a second start migrates nothing and serves the same records', () => {
    expect(o.secondMigrating).toBe(0);
    expect(o.secondComplete).toBe(0);
    expect(o.secondEngine).toBe('rocksdb');
    expect(o.secondHeld).toEqual({ lww: o.oldHeld.lww, or: o.oldHeld.or });
    expect(o.secondStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test('once its TTL has run out the migrated TTL record is hidden and the sweep deletes it', () => {
    expect(o.removed).toBe(1);
    expect(o.expiredRead).toBeUndefined();
    expect(o.expiredLeaves).toEqual({});
  });
});

const JWT_SECRET = 'test-e2e-secret';
const TENANT_CLAIM = 'org_id';
const ADMIN_TOKEN = jwt.sign({ sub: 'operator', roles: ['admin'] }, JWT_SECRET, {
  expiresIn: '1h',
});
const TENANT_RECORDS: Record<string, string[]> = { b: ['b.x', 'b.y'], c: ['c.keep', 'c.two'] };

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

/**
 * One PUT of `{map}` / `k1` by a client of `tenant`; the frame type and, when
 * refused, its code. Sent as a one-op OP_BATCH, because a refused batch is
 * answered with OP_REJECTED while a refused single CLIENT_OP gets no answer.
 */
async function tenantPut(port: number, tenant: string, map: string): Promise<string> {
  const client = await tenantClient(port, tenant);
  try {
    const op = {
      id: `${map}-k1-${Date.now()}`,
      mapName: map,
      opType: 'PUT',
      key: 'k1',
      record: {
        value: { map },
        timestamp: { millis: Date.now(), counter: 0, nodeId: 'engine-upgrade' },
      },
    };
    client.send({ type: 'OP_BATCH', payload: { ops: [op] } });
    const frame = await firstFrame(client, ['OP_ACK', 'OP_REJECTED', 'ERROR'], ACK_WAIT_MS);
    if (!frame) return 'none';
    return frame.type === 'OP_ACK' ? 'OP_ACK' : `${frame.type} ${frame.payload?.code}`;
  } finally {
    client.close();
  }
}

async function tenantRead(port: number, tenant: string, map: string): Promise<unknown> {
  const client = await tenantClient(port, tenant);
  try {
    client.send({ type: 'QUERY_SUB', payload: { queryId: `q-${map}`, mapName: map, query: {} } });
    const response = await firstFrame(client, ['QUERY_RESP'], ACK_WAIT_MS);
    if (!response) return 'no QUERY_RESP';
    const row = (response.payload.results as any[]).find((r) => r.key === 'k1');
    return row ? (row.record?.value ?? row.value) : undefined;
  } finally {
    client.close();
  }
}

describe('Integration: a store with a deleted tenant migrates to RocksDB', () => {
  const o: Record<string, any> = {};
  let dir: string | null = null;
  const running: SpawnedBinary[] = [];
  const tenantEnv = { TOPGUN_TENANT_CLAIM: TENANT_CLAIM };

  beforeAll(async () => {
    const env = requireEngineEnv();
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'topgun-engine-upgrade-tenants-'));

    const old = await spawnBinary({
      binaryPath: env.oldBinary,
      commit: env.oldCommit,
      label: 'OLD tenancy on',
      env: storeEnv(dir, 'redb', tenantEnv),
    });
    running.push(old);
    if (old.port === null) throw new Error(`OLD did not start:\n${old.stderr()}`);
    o.allow = (
      await admin(old.port, 'POST', '/api/admin/policies', {
        id: 'allow-all',
        mapPattern: '**',
        action: 'all',
        effect: 'allow',
      })
    ).status;
    o.writes = {};
    for (const [tenant, maps] of Object.entries(TENANT_RECORDS)) {
      for (const map of maps) o.writes[map] = await tenantPut(old.port, tenant, map);
    }
    await sleep(FLUSH_SETTLE_MS);
    o.deleted = await admin(old.port, 'DELETE', '/api/admin/tenants/b');
    o.oldStop = await drainedStop(old);

    const fresh = await spawnBinary({
      binaryPath: env.newBinary,
      commit: env.newCommit,
      label: 'NEW tenancy on (migrates)',
      env: storeEnv(dir, 'rocksdb', tenantEnv),
      portTimeoutMs: 120_000,
    });
    running.push(fresh);
    if (fresh.port === null) throw new Error(`NEW did not start:\n${fresh.stderr()}`);
    o.complete = linesWith(fresh, COMPLETE_LINE).length;
    const policies = await admin(fresh.port, 'GET', '/api/admin/policies');
    o.policies = (policies.body?.policies ?? []).map((p: any) => p.id).sort();
    o.denyPolicy = (policies.body?.policies ?? []).find((p: any) => p.id === 'tenant-deleted:b');
    const maps = await admin(fresh.port, 'GET', '/api/admin/maps');
    o.mapNames = (maps.body?.maps ?? []).map((m: any) => m.name).sort();
    o.cValues = {};
    for (const map of TENANT_RECORDS.c) o.cValues[map] = await tenantRead(fresh.port, 'c', map);
    o.bWrite = await tenantPut(fresh.port, 'b', 'b.x');
    o.newStop = await drainedStop(fresh);

    process.stderr.write(`[engine-upgrade] tenants, observed:\n${JSON.stringify(o, null, 2)}\n`);
  }, CASE_TIMEOUT_MS);

  afterAll(async () => {
    for (const server of running) await server.stop('SIGKILL', 5_000);
    if (dir) fs.rmSync(dir, { recursive: true, force: true });
  });

  test('setup: every write acked, tenant b deleted by the older binary, a drained stop', () => {
    expect(o.allow).toBe(201);
    expect(o.writes).toEqual({
      'b.x': 'OP_ACK',
      'b.y': 'OP_ACK',
      'c.keep': 'OP_ACK',
      'c.two': 'OP_ACK',
    });
    expect(o.deleted?.status).toBe(200);
    expect(o.deleted?.body?.dropped).toEqual(['b.x', 'b.y']);
    expect(o.oldStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });

  test("after the migration the newer binary serves c's maps and lists nothing of b", () => {
    expect(o.complete).toBe(1);
    expect(o.cValues).toEqual({ 'c.keep': { map: 'c.keep' }, 'c.two': { map: 'c.two' } });
    expect(o.mapNames).toEqual(expect.arrayContaining(['c.keep', 'c.two']));
    expect(o.mapNames.filter((n: string) => n.startsWith('b.'))).toEqual([]);
  });

  test('the tenant-deleted:b deny policy survived the migration and is still in force', () => {
    expect(o.policies).toEqual(['allow-all', 'tenant-deleted:b']);
    expect(o.denyPolicy?.effect).toBe('deny');
    expect(o.bWrite).toBe('OP_REJECTED 403');
    expect(o.newStop).toEqual({ exit: CLEAN_EXIT, drained: true });
  });
});
