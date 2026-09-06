/**
 * Witness for the permanently-rejected-op defect (TODO-662).
 *
 * The defect was found by reading source; this file is the runtime proof, because
 * "read in the source" is not a witness. Every assertion here is written to FAIL
 * once the defect is fixed — they encode the broken behaviour on purpose, so the
 * fix has something concrete to flip.
 *
 * The scenario is the cheapest one a stranger can hit on day one: write to a map
 * the caller's token may not write. The server refuses permanently (retrying can
 * never help), and we observe what the client and the application are told.
 *
 * `Forbidden` is used rather than `ValueTooLarge` because `max_value_bytes`
 * defaults to 0 (unlimited) and the shipped binary exposes no way to set it, so
 * `ValueTooLarge` is unreachable in a real deployment today.
 */

import { TopGunClient } from '@topgunbuild/client';

import { spawnRustServer, createTestToken, SpawnedServer } from './helpers';
import { MemoryStorageAdapter } from './helpers/memory-storage';

const ALLOWED_MAP = 'docs';
const FORBIDDEN_MAP = 'restricted-map';

const adminToken = createTestToken('admin-1', ['admin']);
const userToken = createTestToken('user-1', ['USER']);

/** Reads a Prometheus counter value for the given error label from /metrics. */
async function forbiddenErrorCount(port: number): Promise<number> {
  const resp = await fetch(`http://localhost:${port}/metrics`);
  const body = await resp.text();
  let total = 0;
  for (const line of body.split('\n')) {
    if (line.startsWith('topgun_operation_errors_total') && line.includes('forbidden')) {
      const value = Number(line.trim().split(/\s+/).pop());
      if (Number.isFinite(value)) total += value;
    }
  }
  return total;
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

describe('Witness: permanently-rejected op wedges the client (TODO-662)', () => {
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
    // blamed on the harness rather than on the defect.
    const ok = client.getMap(ALLOWED_MAP);
    ok.set('sanity', { v: 1 });
    await expect(client.confirmWrite(ALLOWED_MAP, 'sanity', 8000)).resolves.toBe('synced');
  });

  afterAll(async () => {
    await client?.close();
    await server?.cleanup();
  });

  test('W1: the refused write is never acked and never leaves the queue', async () => {
    const before = client.getPendingOpsCount();

    client.getMap(FORBIDDEN_MAP).set('k1', { v: 'refused' });

    // Well past any plausible round trip.
    await sleep(3000);

    expect(client.getPendingOpsCount()).toBeGreaterThan(before);
    const pending = await storage.getPendingOps();
    expect(pending.some((op: any) => op.mapName === FORBIDDEN_MAP)).toBe(true);
  });

  test('W2: confirmWrite says "timeout", never "refused" — the honest answer is unavailable', async () => {
    client.getMap(FORBIDDEN_MAP).set('k2', { v: 'refused' });

    const outcome = await client.confirmWrite(FORBIDDEN_MAP, 'k2', 3000);

    // 'timeout' means "not confirmed yet, try again" — advice that is wrong here,
    // because no number of retries can ever make this write succeed. The API that
    // exists precisely to give the honest "did the server take my write?" answer
    // cannot express refusal at all.
    expect(outcome).toBe('timeout');
  });

  test('W3: head-of-line — a legitimate write queued behind it is never acked either', async () => {
    // syncPendingOperations() sends ALL unsynced ops in one OP_BATCH, and the
    // server sends no OP_ACK when any op in the batch fails. So one poisoned op
    // stops the whole queue draining, including writes to maps we may write.
    client.getMap(ALLOWED_MAP).set('behind-the-bad-op', { v: 2 });

    const outcome = await client.confirmWrite(ALLOWED_MAP, 'behind-the-bad-op', 4000);

    expect(outcome).toBe('timeout');
  });

  test('W4: the refused op is re-sent on every subsequent write', async () => {
    const before = await forbiddenErrorCount(server.port);

    // Each new write re-flushes the entire unsynced backlog, so the refused op
    // is re-submitted and refused again, indefinitely.
    for (let i = 0; i < 3; i++) {
      client.getMap(ALLOWED_MAP).set(`churn-${i}`, { i });
      await sleep(400);
    }

    const after = await forbiddenErrorCount(server.port);
    expect(after).toBeGreaterThan(before);
  });

  test('W5: the application is never told — no rejection surfaces anywhere', async () => {
    const rejections: unknown[] = [];
    const unsubscribe = client
      .getConflictResolvers()
      .onRejection((r: unknown) => rejections.push(r));

    client.getMap(FORBIDDEN_MAP).set('k5', { v: 'refused' });
    await sleep(2500);

    unsubscribe();

    // The only trace of a permanently refused write anywhere in the system is a
    // line in our own logger. Nothing reaches the application.
    expect(rejections).toHaveLength(0);
  });
});
