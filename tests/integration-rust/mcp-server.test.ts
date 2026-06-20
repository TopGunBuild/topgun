/**
 * Real-server MCP integration harness (client → Rust server, driven through the
 * built MCP tool handlers).
 *
 * WHY THIS EXISTS — every one of the package's 94 unit tests runs the MCP tools
 * against a `MockTopGunClient` (or a stubbed `WebSocket`) whose `getMap` and
 * `queryOncePaged` read the SAME in-memory store. That unification structurally
 * hides the entire "server-blind read" bug class: a tool that reads the local
 * CRDT replica instead of the server looks identical to one that reads the server,
 * because in the mock they are the same object. This harness boots a REAL Rust
 * server, seeds it from a SEPARATE client, then drives the real
 * `TopGunMCPServer.callTool()` against a COLD MCP client (its own cache has never
 * seen the seeded data) — the only configuration in which a server-blind read is
 * observably wrong.
 *
 * NEGATIVE CONTROL — these assertions are written for the CORRECT (server-
 * authoritative) behavior. Against the pre-fix code, where `topgun_schema`,
 * `topgun_stats`, and `topgun_explain` iterate `client.getMap(map).entries()`,
 * the cold MCP cache is empty, so they report "Map is empty / Records: 0 /
 * Total Records: 0" and every assertion below FAILS. That red is the proof the
 * bug class is now visible to CI; the fix (reads routed through `queryOncePaged`)
 * turns it green.
 */

import { TopGunClient, SyncState } from '@topgunbuild/client';
import type { IStorageAdapter, OpLogEntry } from '@topgunbuild/client';
import { TopGunMCPServer } from '@topgunbuild/mcp-server';

import { spawnRustServer, createTestToken, SpawnedServer } from './helpers';

// In-memory storage adapter so the SDK runs headless in Node (no IndexedDB).
class MemoryStorageAdapter implements IStorageAdapter {
  private kv = new Map<string, unknown>();
  private meta = new Map<string, unknown>();
  private opLog: OpLogEntry[] = [];
  private pending: OpLogEntry[] = [];

  async initialize(): Promise<void> {}
  async close(): Promise<void> {}
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  async get(key: string): Promise<any> {
    return this.kv.get(key);
  }
  async put(key: string, value: unknown): Promise<void> {
    this.kv.set(key, value);
  }
  async remove(key: string): Promise<void> {
    this.kv.delete(key);
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    this.meta.set(key, value);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<OpLogEntry, 'id'>): Promise<number> {
    const id = this.opLog.length + 1;
    const e = { ...entry, id, synced: 0 } as OpLogEntry;
    this.opLog.push(e);
    this.pending.push(e);
    return id;
  }
  async getPendingOps(): Promise<OpLogEntry[]> {
    return this.pending;
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.pending = this.pending.filter((op) => (op.id ?? 0) > lastId);
    this.opLog.forEach((op) => {
      if ((op.id ?? 0) <= lastId) op.synced = 1;
    });
  }
  async deleteOp(id: number): Promise<void> {
    this.opLog = this.opLog.filter((op) => op.id !== id);
  }
  async commitWrite(
    mutations: Array<{
      store: 'kv' | 'meta';
      type: 'put' | 'remove';
      key: string;
      value?: unknown;
    }>,
    op: Omit<OpLogEntry, 'id'>,
  ): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.kv;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value as never);
    }
    return this.appendOpLog(op);
  }
  async getAllKeys(): Promise<string[]> {
    return Array.from(this.kv.keys());
  }
}

function waitMs(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForState(
  client: TopGunClient,
  state: SyncState,
  timeoutMs = 20_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (client.getConnectionState() === state) return;
    await waitMs(50);
  }
  throw new Error(`Timed out waiting for ${state}; last = ${client.getConnectionState()}`);
}

/** Resolves once every optimistic local write has round-tripped and been ACKed. */
async function waitForSynced(client: TopGunClient, timeoutMs = 20_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (client.getPendingOpsCount() === 0) return;
    await waitMs(25);
  }
  throw new Error(`Timed out waiting to drain; pending = ${client.getPendingOpsCount()}`);
}

function text(result: { content: Array<{ text?: string }> }): string {
  return (result.content || []).map((c) => c.text ?? '').join('\n');
}

// Both clients authenticate with a JWT (same pattern the other single-server
// integration suites use), rather than relying on a NO_AUTH server. The tokenless
// NO_AUTH path is racy for a second connecting client under CI load (the WS
// upgrade intermittently fails before the grace window resolves); explicit auth
// is the deterministic, CI-proven path.
function makeClient(port: number, userId: string): TopGunClient {
  const token = createTestToken(userId, ['ADMIN']);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

const RECORD_COUNT = 25;
const OPEN_COUNT = 10; // records 0..9 are status:'open', 10..24 are status:'done'

describe('Integration: MCP tools against a real Rust server (cold cache)', () => {
  let server: SpawnedServer | null = null;
  let seeder: TopGunClient | null = null;
  let mcp: TopGunMCPServer | null = null;
  let mapName = '';

  beforeEach(async () => {
    server = await spawnRustServer();

    // 1. Seeder client — writes data the MCP process's cache has NEVER seen.
    seeder = makeClient(server.port, 'mcp-seeder');
    await seeder.start();
    await waitForState(seeder, SyncState.CONNECTED);

    // Unique map per test so the null in-memory backend never bleeds rows across
    // runs and every seeded row is unambiguously part of this test's dataset.
    mapName = `mcp_e2e_${Date.now()}`;
    const m = seeder.getMap<string, Record<string, unknown>>(mapName);
    for (let i = 0; i < RECORD_COUNT; i++) {
      m.set(`k${i}`, {
        id: `k${i}`,
        title: `doc ${i}`,
        status: i < OPEN_COUNT ? 'open' : 'done',
        n: i,
      });
    }
    await waitForSynced(seeder);

    // 2. MCP server driving a SEPARATE, freshly-built client (cold cache) → same
    // server. We inject the client so it carries its own auth, and start it
    // directly rather than via mcp.start() (which would attach a stdio transport
    // that consumes the Jest process's stdin); callTool() only needs the client
    // connected.
    const mcpClient = makeClient(server.port, 'mcp-reader');
    mcp = new TopGunMCPServer({ client: mcpClient });
    await mcpClient.start();
    await waitForState(mcpClient, SyncState.CONNECTED);
  }, 60_000);

  afterEach(async () => {
    if (mcp) {
      await mcp
        .getClient()
        .close()
        .catch(() => {});
      mcp = null;
    }
    if (seeder) {
      await seeder.close().catch(() => {});
      seeder = null;
    }
    if (server) {
      await server.cleanup().catch(() => {});
      server = null;
    }
  });

  test('topgun_query returns the seeded rows (server-authoritative sanity)', async () => {
    const result = await mcp!.callTool('topgun_query', { map: mapName, limit: 5 });
    expect(result.isError).toBeFalsy();
    const out = text(result);
    expect(out).toContain('Found 5 result(s)');
    expect(out).toContain('doc');
  });

  // --- F1: server-blind read divergence is gone (the negative control) -------
  //
  // Each of these reads happens on a COLD MCP client with NO prior topgun_mutate
  // or topgun_query in this process — so the local replica is empty. A correct
  // (server-authoritative) tool still sees all 25 seeded rows; the pre-fix
  // local-replica reads would report empty/0 here and fail.

  test('topgun_schema (cold) reflects the 25 server records, not an empty map', async () => {
    const result = await mcp!.callTool('topgun_schema', { map: mapName });
    expect(result.isError).toBeFalsy();
    const out = text(result);
    expect(out).not.toMatch(/is empty/i);
    expect(out).toContain('Records: 25');
    // Fields inferred from real server rows.
    expect(out).toContain('title');
    expect(out).toContain('status');
    expect(out).toContain('n');
  });

  test('topgun_stats (cold) reports the 25-record server count', async () => {
    const result = await mcp!.callTool('topgun_stats', { map: mapName });
    expect(result.isError).toBeFalsy();
    const out = text(result);
    expect(out).toContain(mapName);
    expect(out).toContain('Records: 25');
  });

  test('topgun_explain (cold) plans over real server totals and selectivity', async () => {
    const result = await mcp!.callTool('topgun_explain', {
      map: mapName,
      filter: { status: 'open' },
    });
    expect(result.isError).toBeFalsy();
    const out = text(result);
    expect(out).toContain('Total Records: 25');
    expect(out).toContain(`Estimated Results: ${OPEN_COUNT}`);
    expect(out).toContain('Selectivity: 40.0%');
  });

  test('cold schema sees server data BEFORE any query hydrates the local cache', async () => {
    // The pre-fix bug persisted even AFTER a query (queryOncePaged never wrote the
    // persistent replica). Here we prove the stronger property: the very FIRST
    // read on a cold process already reflects the server, with no warm-up.
    const schema = await mcp!.callTool('topgun_schema', { map: mapName });
    expect(text(schema)).toContain('Records: 25');

    // And it stays consistent after a query, too.
    await mcp!.callTool('topgun_query', { map: mapName, limit: 5 });
    const schemaAfter = await mcp!.callTool('topgun_schema', { map: mapName });
    expect(text(schemaAfter)).toContain('Records: 25');
  });
});
