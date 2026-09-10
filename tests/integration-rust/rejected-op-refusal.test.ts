/**
 * End-to-end proof that a write the server will never accept is refused
 * cleanly, per operation, instead of wedging the client.
 *
 * This file is the inversion of the witness that used to live here: the same
 * scenario, the same five observations, every assertion flipped. The witness
 * encoded the broken behaviour on purpose so the fix had something concrete to
 * flip; what follows is what the application is entitled to see instead.
 *
 * The scenario is the cheapest one a stranger can hit on day one: write to a
 * map the caller's token may not write. The server refuses permanently (no
 * number of retries can help), names the operation, and the client retires that
 * operation alone — the queue keeps draining and the application is told.
 *
 * `Forbidden` is used rather than `ValueTooLarge` because `max_value_bytes`
 * defaults to 0 (unlimited) and the shipped binary exposes no way to set it, so
 * `ValueTooLarge` is unreachable in a real deployment today.
 */

import { TopGunClient } from '@topgunbuild/client';
import type { WriteRejection } from '@topgunbuild/core';

import { spawnRustServer, createTestToken, SpawnedServer } from './helpers';
import { MemoryStorageAdapter } from './helpers/memory-storage';

const ALLOWED_MAP = 'docs';
const FORBIDDEN_MAP = 'restricted-map';

const adminToken = createTestToken('admin-1', ['admin']);
const userToken = createTestToken('user-1', ['USER']);

/**
 * Reads the refusal counter for one `{transport, reason}` series off `/metrics`.
 *
 * Returns `null` when the series is absent, which is a distinct answer from
 * `0`: the server registers the whole label space eagerly on connection setup,
 * so a missing line means the instrument itself is broken, and silently reading
 * that as a zero is how a broken instrument passes for a healthy server.
 */
async function refusalCount(
  port: number,
  transport: string,
  reason: string,
): Promise<number | null> {
  const resp = await fetch(`http://localhost:${port}/metrics`);
  const body = await resp.text();
  for (const line of body.split('\n')) {
    if (
      line.startsWith('topgun_client_op_refusals_total') &&
      line.includes(`transport="${transport}"`) &&
      line.includes(`reason="${reason}"`)
    ) {
      const value = Number(line.trim().split(/\s+/).pop());
      if (Number.isFinite(value)) return value;
    }
  }
  return null;
}

async function createPolicy(port: number, id: string, mapPattern: string): Promise<void> {
  const resp = await fetch(`http://localhost:${port}/api/admin/policies`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${adminToken}` },
    body: JSON.stringify({ id, mapPattern, action: 'write', effect: 'allow' }),
  });
  if (!resp.ok) throw new Error(`policy ${id} failed: ${resp.status} ${await resp.text()}`);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('rejected-op-refusal: a permanently refused op is retired, not retried', () => {
  let server: SpawnedServer;
  let storage: MemoryStorageAdapter;
  let client: TopGunClient;

  beforeAll(async () => {
    server = await spawnRustServer({ env: { TOPGUN_ADMIN_SUBJECTS: 'admin-1' } });
    // Only ALLOWED_MAP is writable. Any other map falls to default-deny for an
    // authenticated caller, which is what makes FORBIDDEN_MAP permanently refused.
    await createPolicy(server.port, 'allow-docs', ALLOWED_MAP);

    storage = new MemoryStorageAdapter();
    client = new TopGunClient({
      serverUrl: `ws://localhost:${server.port}/ws`,
      storage,
      auth: { getToken: async () => userToken },
    });
    await client.start();

    // Sanity: the allowed map really is writable, so a later failure cannot be
    // blamed on the harness rather than on the behaviour under test.
    const ok = client.getMap(ALLOWED_MAP);
    ok.set('sanity', { v: 1 });
    await expect(client.confirmWrite(ALLOWED_MAP, 'sanity', 8000)).resolves.toBe('synced');
  });

  afterAll(async () => {
    await client?.close();
    await server?.cleanup();
  });

  test('R1: the refused write leaves the queue, and the local value is kept', async () => {
    const before = client.getPendingOpsCount();

    client.getMap(FORBIDDEN_MAP).set('k1', { v: 'refused' });

    // Well past any plausible round trip.
    await sleep(3000);

    expect(client.getPendingOpsCount()).toBe(before);
    const pending = await storage.getPendingOps();
    expect(pending.some((op: { mapName?: string }) => op.mapName === FORBIDDEN_MAP)).toBe(false);

    // Keep-and-present: a refusal is reported, never rolled back, so the value
    // the caller wrote is still readable locally. A rollback here would destroy
    // the user's data on a server decision they have not yet seen.
    expect(client.getMap(FORBIDDEN_MAP).get('k1')).toEqual({ v: 'refused' });
  });

  test('R2: confirmWrite answers "rejected" — the honest answer is now available', async () => {
    client.getMap(FORBIDDEN_MAP).set('k2', { v: 'refused' });

    const outcome = await client.confirmWrite(FORBIDDEN_MAP, 'k2', 3000);

    // Not 'timeout'. 'timeout' means "not confirmed yet, try again" — advice
    // that is wrong here, because no number of retries can make this write
    // succeed. 'rejected' is the terminal answer the caller can act on.
    expect(outcome).toBe('rejected');
  });

  test('R3: no head-of-line block — a legitimate write behind it is acked', async () => {
    // syncPendingOperations() still sends the whole backlog in one OP_BATCH, but
    // the server now judges each op on its own: the refused one is named in an
    // OP_REJECTED and the rest are acked, so one poisoned op no longer stops the
    // queue draining.
    client.getMap(ALLOWED_MAP).set('behind-the-bad-op', { v: 2 });

    const outcome = await client.confirmWrite(ALLOWED_MAP, 'behind-the-bad-op', 4000);

    expect(outcome).toBe('synced');
  });

  test('R4: the refusal is counted exactly once and the op is never re-sent', async () => {
    // The series is registered eagerly at connection setup, so it must already
    // be present here — present-and-zero, not absent.
    const before = await refusalCount(server.port, 'ws', 'forbidden');
    expect(before).not.toBeNull();

    client.getMap(FORBIDDEN_MAP).set('k4', { v: 'refused' });
    await sleep(2500);

    // (a) The refusal moved its own series by exactly one. Asserting on this
    // series rather than on topgun_operation_errors_total is deliberate: the
    // latter counts failed pipeline calls and rises by two for one refused op.
    const afterRefusal = await refusalCount(server.port, 'ws', 'forbidden');
    expect(afterRefusal).toBe((before as number) + 1);

    // (b) And it stays there. Each new write re-flushes the backlog; if the
    // refused op were still in it, it would be refused again and the counter
    // would climb. Half (a) is what stops this half being vacuously true.
    for (let i = 0; i < 3; i++) {
      client.getMap(ALLOWED_MAP).set(`churn-${i}`, { i });
      await sleep(400);
    }

    expect(await refusalCount(server.port, 'ws', 'forbidden')).toBe(afterRefusal);
  });

  test('R5: the application is told, once, with a machine-readable cause', async () => {
    const rejections: WriteRejection[] = [];
    const unsubscribe = client.onWriteRejected((r) => rejections.push(r));

    client.getMap(FORBIDDEN_MAP).set('k5', { v: 'refused' });
    await sleep(2500);

    unsubscribe();

    const forThisWrite = rejections.filter((r) => r.mapName === FORBIDDEN_MAP && r.key === 'k5');
    expect(forThisWrite).toHaveLength(1);
    expect(forThisWrite[0].cause).toBe('forbidden');
    expect(forThisWrite[0].reason).toEqual(expect.any(String));
    expect(forThisWrite[0].reason.length).toBeGreaterThan(0);
    expect(forThisWrite[0].keptLocally).toBe(true);
  });
});
