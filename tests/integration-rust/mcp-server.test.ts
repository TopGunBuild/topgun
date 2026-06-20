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
import type { MCPToolResult } from '@topgunbuild/mcp-server';

import {
  spawnRustServer,
  createRustTestClient,
  createTestToken,
  createLWWRecord,
  completeMerkleSync,
} from './helpers';

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

function text(result: { content: Array<{ text?: string }> }): string {
  return (result.content || []).map((c) => c.text ?? '').join('\n');
}

// Parse the head line of each change the subscribe feed renders, e.g.
//   "1. [2026-...Z] ADD (entered result set) - live1"
//   "2. [2026-...Z] REMOVE (deleted / left result set) - k0"
// JSON body lines (indented) never match, so only one record per delta is counted.
function parseDeltas(out: string): Array<{ kind: string; key: string }> {
  const re = /^\s*\d+\.\s+\[[^\]]+\]\s+(ADD|UPDATE|REMOVE)\b.*?-\s+(\S+)\s*$/gm;
  const deltas: Array<{ kind: string; key: string }> = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(out)) !== null) {
    deltas.push({ kind: m[1], key: m[2] });
  }
  return deltas;
}

// Both clients authenticate with a JWT (same pattern the other single-server
// integration suites use), rather than relying on a NO_AUTH server. The tokenless
// NO_AUTH path is racy for a second connecting client under CI load (the WS
// upgrade intermittently fails before the grace window resolves); explicit auth
// is the deterministic, CI-proven path.
function makeClient(port: number, userId: string, roles: string[] = ['ADMIN']): TopGunClient {
  const token = createTestToken(userId, roles);
  return new TopGunClient({
    serverUrl: `ws://localhost:${port}/ws`,
    storage: new MemoryStorageAdapter(),
    auth: { getToken: async () => token },
    backoff: { initialDelayMs: 200, maxDelayMs: 400, jitter: true },
  });
}

const RECORD_COUNT = 25;
const OPEN_COUNT = 10; // records 0..9 are status:'open', 10..24 are status:'done'

// Retry transient connectivity failures (NOT content mismatches): the read tools
// go through queryOncePaged, which rejects if the client is momentarily
// reconnecting. Re-confirm CONNECTED and retry, so a brief blip never reds the
// suite. A genuine server-blind read (the pre-fix "Map is empty" / "Records: 0")
// is NOT a transient error, so it is never retried — the negative control still
// fails fast against the unfixed tools.
const TRANSIENT = /unreachable|not settled|did not settle|client offline/i;
function isTransientConnectivity(result: MCPToolResult): boolean {
  const out = text(result);
  // queryOncePaged-backed tools (query/schema/explain) surface offline/unsettled
  // as an isError result — gate on isError so a SUCCESSFUL response whose record
  // content merely contains one of these words is never retried.
  if (result.isError && TRANSIENT.test(out)) return true;
  // topgun_stats stays isError=false and instead degrades only its per-map line;
  // match that exact marker so an unreachable stats read is retried too.
  return /Records: unavailable \(server unreachable/i.test(out);
}
async function callStable(
  mcp: TopGunMCPServer,
  name: string,
  args: unknown,
): Promise<MCPToolResult> {
  let last: MCPToolResult | undefined;
  for (let attempt = 0; attempt < 6; attempt++) {
    await waitForState(mcp.getClient(), SyncState.CONNECTED, 8_000).catch(() => {});
    last = await mcp.callTool(name, args);
    if (!isTransientConnectivity(last)) return last;
    await waitMs(500);
  }
  return last!;
}

describe('Integration: MCP tools against a real Rust server (cold cache)', () => {
  // One self-contained test that boots the server, seeds, and runs every read
  // back-to-back. Doing the whole scenario inline keeps it FAST (a few seconds),
  // the lifecycle the reliably-green single-server suites use.
  //
  // The SEEDER is the raw standalone test-client (CLIENT_OP over the wire), NOT a
  // second TopGunClient. Two TopGunClients each run heartbeat + Merkle-sync and an
  // aggressive auto-reconnect; under CI load a single blip cascaded into a non-101
  // reconnect storm that left the reader offline for the rest of the suite. The
  // raw client has no reconnect/sync machinery (it just PINGs to stay past the
  // server idle reaper), so it holds a stable connection with no storm — and it
  // keeps the map's rows live on the null in-memory backend (which drops them once
  // no client holds the map). Seeding over the wire also never touches the MCP
  // client's local replica, so the reader stays genuinely COLD: it never mutates
  // and queryOncePaged does not hydrate the persistent replica, so its local CRDT
  // map is empty for every read — exactly the configuration that exposes a
  // server-blind read. We inject the reader client so it carries its own auth, and
  // start it directly rather than via mcp.start() (which would attach a stdio
  // transport that consumes the Jest process's stdin); callTool() only needs it
  // connected.
  test('cold MCP read tools reflect real server data, not an empty local replica (F1)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-reader');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      // Seed over the wire (raw CLIENT_OP PUTs), waiting for each server ACK.
      const mapName = `mcp_e2e_${Date.now()}`;
      for (let i = 0; i < RECORD_COUNT; i++) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${i}`,
            mapName,
            opType: 'PUT',
            key: `k${i}`,
            record: createLWWRecord({
              id: `k${i}`,
              title: `doc ${i}`,
              status: i < OPEN_COUNT ? 'open' : 'done',
              n: i,
            }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      // Sanity: topgun_query (already server-authoritative) returns the rows.
      const query = await callStable(mcp, 'topgun_query', { map: mapName, limit: 5 });
      expect(query.isError).toBeFalsy();
      expect(text(query)).toContain('Found 5 result(s)');
      expect(text(query)).toContain('doc');

      // F1 — each read runs on a COLD MCP client (no prior mutate/query in this
      // process, so the local replica is empty). A correct server-authoritative
      // tool still sees all 25 seeded rows; the pre-fix local-replica reads would
      // report "empty / 0" here and fail (the negative control).
      const schema = await callStable(mcp, 'topgun_schema', { map: mapName });
      expect(schema.isError).toBeFalsy();
      expect(text(schema)).not.toMatch(/is empty/i);
      expect(text(schema)).toContain('Records: 25');
      expect(text(schema)).toContain('title');
      expect(text(schema)).toContain('status');
      expect(text(schema)).toContain('n');

      const stats = await callStable(mcp, 'topgun_stats', { map: mapName });
      expect(stats.isError).toBeFalsy();
      expect(text(stats)).toContain(mapName);
      expect(text(stats)).toContain('Records: 25');

      const explain = await callStable(mcp, 'topgun_explain', {
        map: mapName,
        filter: { status: 'open' },
      });
      expect(explain.isError).toBeFalsy();
      expect(text(explain)).toContain('Total Records: 25');
      expect(text(explain)).toContain(`Estimated Results: ${OPEN_COUNT}`);
      expect(text(explain)).toContain('Selectivity: 40.0%');

      // A query does not hydrate the persistent replica these tools read (that
      // was the bug), so schema stays correct (server-derived) after one.
      await callStable(mcp, 'topgun_query', { map: mapName, limit: 5 });
      const schemaAfter = await callStable(mcp, 'topgun_schema', { map: mapName });
      expect(text(schemaAfter)).toContain('Records: 25');
    } finally {
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);

  // F2 — topgun_mutate{set} must reflect SERVER-confirmed state, never an
  // optimistic local echo. We assert the write is reported successful only after
  // the server applied it, and prove that by reading it back from a SEPARATE
  // client (the raw seeder, over the wire) — the MCP process's own local replica
  // is irrelevant to whether the data is durable on the server.
  //
  // F5 — topgun_mutate{remove} must be server-authoritative: it deletes a record
  // that exists on the server but is absent from the cold MCP cache, and the
  // deletion is visible to a subsequent server read. Against the pre-fix code the
  // remove short-circuited on the empty local cache ("does not exist, no action"),
  // so the record survived — these assertions are the negative control for that.
  test('mutate is server-confirmed (F2) and remove is server-authoritative (F5)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-writer');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      // ---- F2: set, confirmed on the server, visible to a separate client ----
      const writeMap = `mcp_write_${Date.now()}`;
      const setRes = await callStable(mcp, 'topgun_mutate', {
        map: writeMap,
        operation: 'set',
        key: 'w1',
        data: { title: 'hello', n: 1 },
      });
      expect(setRes.isError).toBeFalsy();
      expect(text(setRes)).toContain('Successfully saved');
      expect(text(setRes)).toContain('confirmed on server');

      // A SEPARATE client pulls the map from the server via Merkle sync and sees
      // the record — proof the write is durable server-side, not local-only.
      const seenAfterSet = await completeMerkleSync(seeder, writeMap);
      expect(seenAfterSet.has('w1')).toBe(true);
      expect(seenAfterSet.get('w1')?.value?.title).toBe('hello');

      // ---- F5: remove a server-resident key that is COLD to the MCP cache ----
      const removeMap = `mcp_remove_${Date.now()}`;
      // Seed two keys over the wire (server-resident; the MCP client never wrote
      // them, so they are absent from its local replica). 'keep1' is a sentinel to
      // prove the remove is targeted, not a blanket wipe.
      for (const [k, title] of [
        ['doomed', 'delete me'],
        ['keep1', 'survivor'],
      ]) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${removeMap}-${k}`,
            mapName: removeMap,
            opType: 'PUT',
            key: k,
            record: createLWWRecord({ id: k, title }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      // The MCP client has never cached 'doomed' — a pre-fix remove would no-op
      // with "does not exist". The fix issues the tombstone server-authoritatively.
      const removeRes = await callStable(mcp, 'topgun_mutate', {
        map: removeMap,
        operation: 'remove',
        key: 'doomed',
      });
      expect(removeRes.isError).toBeFalsy();
      expect(text(removeRes)).toContain('Successfully removed');
      expect(text(removeRes)).toContain('confirmed on server');
      expect(text(removeRes)).not.toMatch(/does not exist/i);

      // Server-authoritative read-back: 'doomed' is gone, 'keep1' remains.
      const query = await callStable(mcp, 'topgun_query', { map: removeMap, limit: 50 });
      expect(query.isError).toBeFalsy();
      expect(text(query)).toContain('survivor');
      expect(text(query)).not.toContain('delete me');
    } finally {
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);

  // Negative control for F2: with no reachable server, a mutate must NOT report
  // success. The pre-fix tool answered "Successfully created" optimistically (and
  // the InMemory adapter then lost the write on exit); the fix returns an explicit
  // not-durable error for both set and remove.
  test('mutate returns an explicit not-durable error when the server is unreachable (F2 negative control)', async () => {
    // Point at a port with nothing listening so the client never connects.
    const deadPort = 59_237;
    const mcpClient = makeClient(deadPort, 'mcp-offline');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await mcpClient.start();

      const setRes = await mcp.callTool('topgun_mutate', {
        map: 'offline_map',
        operation: 'set',
        key: 'k1',
        data: { x: 1 },
      });
      expect(setRes.isError).toBe(true);
      expect(text(setRes)).toMatch(/not yet.*durable/i);
      expect(text(setRes)).not.toContain('Successfully');

      const removeRes = await mcp.callTool('topgun_mutate', {
        map: 'offline_map',
        operation: 'remove',
        key: 'k1',
      });
      expect(removeRes.isError).toBe(true);
      expect(text(removeRes)).toMatch(/not yet.*durable/i);
      expect(text(removeRes)).not.toContain('Successfully');
    } finally {
      await mcpClient.close().catch(() => {});
    }
  }, 30_000);

  // F3 — topgun_subscribe must be a REAL change feed: one correctly-typed delta
  // per genuine change, removals included, and NOT a re-dump of the whole
  // snapshot. The pre-fix tool blocked for the timeout and emitted the full
  // result set as `UPDATE` "changes" on every membership re-delivery (a 2-record
  // scenario produced 103 changes, all UPDATE; the real removal was invisible).
  //
  // We seed N rows, open the feed (action:'start' — returns at once), then make
  // exactly two concurrent changes via a SEPARATE client over the wire: one
  // brand-new key (an ADD) and one removal of a pre-existing key (a REMOVE).
  // Draining the feed (action:'poll') must yield EXACTLY those two deltas with
  // the correct types — never the untouched pre-existing rows.
  test('subscribe is a real change feed: one typed delta per change, removals visible (F3)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-watcher');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      // Seed a pre-existing dataset (k0..k9) and wait for each server ACK so the
      // feed's baseline snapshot already contains all of them — none must later
      // surface as a "change".
      const mapName = `mcp_sub_${Date.now()}`;
      const PRE_COUNT = 10;
      for (let i = 0; i < PRE_COUNT; i++) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${i}`,
            mapName,
            opType: 'PUT',
            key: `k${i}`,
            record: createLWWRecord({ id: `k${i}`, title: `doc ${i}`, n: i }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      // Open the change feed. Returns immediately with a subscriptionId (does NOT
      // block for a timeout) once the server baseline settles.
      const startRes = await callStable(mcp, 'topgun_subscribe', {
        action: 'start',
        map: mapName,
      });
      expect(startRes.isError).toBeFalsy();
      const idMatch = text(startRes).match(/subscriptionId:\s*([0-9a-f-]+)\)/i);
      expect(idMatch).toBeTruthy();
      const subscriptionId = idMatch![1];

      // An immediate poll before any change must report nothing — the baseline
      // snapshot is NOT reported as changes (the core pre-fix defect).
      const emptyPoll = await mcp.callTool('topgun_subscribe', {
        action: 'poll',
        subscriptionId,
      });
      expect(emptyPoll.isError).toBeFalsy();
      expect(text(emptyPoll)).toMatch(/no changes/i);

      // Make EXACTLY two concurrent changes over the wire:
      //   1. a brand-new key 'live1'  → ADD
      //   2. a removal of existing 'k0' → REMOVE
      seeder.messages.length = 0;
      seeder.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-add',
          mapName,
          opType: 'PUT',
          key: 'live1',
          record: createLWWRecord({ id: 'live1', title: 'fresh', n: 999 }),
        },
      });
      await seeder.waitForMessage('OP_ACK', 10_000);

      seeder.messages.length = 0;
      seeder.send({
        type: 'CLIENT_OP',
        payload: {
          id: 'live-remove',
          mapName,
          opType: 'REMOVE',
          key: 'k0',
          record: createLWWRecord(null),
        },
      });
      await seeder.waitForMessage('OP_ACK', 10_000);

      // Drain the feed. Deltas may arrive across more than one server push, so we
      // poll until both expected changes are seen, then poll a few more times to
      // prove no EXTRA deltas (e.g. a re-dumped snapshot row) follow.
      const collected: Array<{ kind: string; key: string }> = [];
      const deadline = Date.now() + 15_000;
      let extraPollsAfterComplete = 0;
      while (Date.now() < deadline) {
        const poll = await mcp.callTool('topgun_subscribe', { action: 'poll', subscriptionId });
        expect(poll.isError).toBeFalsy();
        collected.push(...parseDeltas(text(poll)));
        if (collected.length >= 2) {
          // Got the two we expect — confirm the feed is now quiet (no snapshot dump).
          if (++extraPollsAfterComplete >= 3) break;
        }
        await waitMs(400);
      }

      // EXACTLY two deltas — not 103, not the pre-existing rows re-reported.
      expect(collected).toHaveLength(2);

      const add = collected.find((c) => c.key === 'live1');
      const remove = collected.find((c) => c.key === 'k0');
      expect(add?.kind).toBe('ADD'); // new record entered the result set
      expect(remove?.kind).toBe('REMOVE'); // removal is VISIBLE as its own delta

      // Negative control: none of the untouched pre-existing rows (k1..k9) are
      // ever reported — the snapshot is not dumped as "changes".
      for (let i = 1; i < PRE_COUNT; i++) {
        expect(collected.some((c) => c.key === `k${i}`)).toBe(false);
      }

      // Tearing down is explicit and acknowledged.
      const stopRes = await mcp.callTool('topgun_subscribe', { action: 'stop', subscriptionId });
      expect(stopRes.isError).toBeFalsy();
      expect(text(stopRes)).toMatch(/stopped/i);
    } finally {
      await mcp.stop().catch(() => {});
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);

  // F4 — topgun_list_maps must reflect the REAL server catalog, never fabricated
  // "common patterns". A separate seeder creates two distinctly-named maps over
  // the wire; list_maps (server-authoritative, via the admin map-enumeration
  // endpoint) must return exactly those names with their entry counts — and never
  // the retired boilerplate ('users'/'tasks'/'posts'/'products' as examples). The
  // MCP client carries both the data-plane 'ADMIN' role (to connect) and the
  // admin-plane 'admin' role (the HTTP control plane requires it).
  test('list_maps reflects the real server catalog, not fabricated names (F4)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-lister', ['ADMIN', 'admin']);
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      // Seed two distinctly-named maps (one row each) over the wire. The unique
      // suffix keeps the assertion robust against any residue from other suites.
      const suffix = Date.now();
      const mapA = `mcp_catalog_a_${suffix}`;
      const mapB = `mcp_catalog_b_${suffix}`;
      for (const map of [mapA, mapB]) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${map}`,
            mapName: map,
            opType: 'PUT',
            key: 'k0',
            record: createLWWRecord({ id: 'k0', title: 'row' }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      const result = await callStable(mcp, 'topgun_list_maps', {});
      expect(result.isError).toBeFalsy();
      const out = text(result);

      // The real, server-resident maps are listed, with entry counts.
      expect(out).toContain(mapA);
      expect(out).toContain(mapB);
      expect(out).toMatch(new RegExp(`${mapA}\\s*\\(1 entry`));

      // Negative control: the pre-fix fabricated "common patterns" are gone. The
      // server never created these names, so a server-authoritative answer for a
      // catalog made only of mapA/mapB can never mention them.
      expect(out).not.toMatch(/common (map )?pattern/i);
      expect(out).not.toContain('Blog posts');
      expect(out).not.toContain('E-commerce products');
    } finally {
      await mcp.stop().catch(() => {});
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);

  // F4 negative control — with no reachable server, list_maps must say so
  // honestly (DISCONNECTED), never emit a cheerful fabricated catalog.
  test('list_maps reports DISCONNECTED honestly when the server is unreachable (F4 negative control)', async () => {
    const deadPort = 59_241;
    const mcpClient = makeClient(deadPort, 'mcp-list-offline', ['ADMIN', 'admin']);
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await mcpClient.start();

      const result = await mcp.callTool('topgun_list_maps', {});
      expect(result.isError).toBe(true);
      expect(text(result)).toMatch(/not connected|NOT an empty/i);
      expect(text(result)).not.toContain('Blog posts');
      expect(text(result)).not.toMatch(/common (map )?pattern/i);
    } finally {
      await mcpClient.close().catch(() => {});
    }
  }, 30_000);

  // F6 — topgun_search must hydrate hit bodies SERVER-AUTHORITATIVELY. hybridSearch
  // ranks keys on the server but returns no bodies; the MCP process's local CRDT
  // replica is cold for any record it did not write itself. The pre-fix tool read
  // the body from that cold replica, so every hit rendered "(record body not
  // available locally)" against real server data. We seed FTS-indexable docs from a
  // SEPARATE client over the wire, then search from a COLD MCP client and assert the
  // real server body text appears — the only configuration that exposes the bug.
  //
  // hybrid search additionally gates on the map having a registered index registry
  // (the real "index registry not found" path); we register it via the admin
  // index endpoint so the fullText (tantivy) leg actually returns ranked hits.
  test('search hydrates hit bodies from the server, not the cold local replica (F6)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-searcher');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      // Seed articles with distinctive, FTS-indexable body text over the wire.
      const mapName = `mcp_search_${Date.now()}`;
      const articles: Array<{ key: string; title: string; body: string }> = [
        {
          key: 'art-1',
          title: 'Introduction to Machine Learning',
          body: 'Machine learning is a subset of artificial intelligence.',
        },
        {
          key: 'art-2',
          title: 'Deep Learning Fundamentals',
          body: 'Deep learning uses neural networks with many layers.',
        },
        {
          key: 'art-3',
          title: 'Advanced Machine Learning Techniques',
          body: 'Covers advanced machine learning algorithms and optimization.',
        },
      ];
      for (const art of articles) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${art.key}`,
            mapName,
            opType: 'PUT',
            key: art.key,
            record: createLWWRecord({ id: art.key, title: art.title, body: art.body }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      // Register the map's index registry so hybrid search clears its
      // "index registry not found" gate and the tantivy fullText leg runs. Requires
      // an admin-plane ('admin') JWT — distinct from the data-plane 'ADMIN' role.
      const adminToken = createTestToken('mcp-index-admin', ['admin']);
      const createIndexResp = await fetch(`http://localhost:${server.port}/api/admin/indexes`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${adminToken}`,
        },
        body: JSON.stringify({ mapName, attribute: 'title', indexType: 'inverted' }),
      });
      expect(createIndexResp.status).toBe(201);

      // Poll the search until the full-text index has caught up (lazy/batched
      // indexing). A genuine server-blind read is NOT a "no results" case, so this
      // loop never masks the negative control: once hits arrive, their bodies must
      // be the real server bodies.
      let result: MCPToolResult | undefined;
      const deadline = Date.now() + 20_000;
      while (Date.now() < deadline) {
        await waitForState(mcp.getClient(), SyncState.CONNECTED, 8_000).catch(() => {});
        result = await mcp.callTool('topgun_search', {
          map: mapName,
          query: 'machine learning',
        });
        if (!result.isError && /Found \d+ result/.test(text(result))) break;
        await waitMs(500);
      }

      expect(result!.isError).toBeFalsy();
      const out = text(result!);
      expect(out).toMatch(/Found \d+ result/);
      // The hit bodies are the REAL server bodies, hydrated server-authoritatively.
      expect(out).toContain('Machine Learning');
      expect(out).toMatch(/subset of artificial intelligence|machine learning algorithms/);
      // Negative control: the pre-fix cold-cache markers must never appear.
      expect(out).not.toContain('not available locally');
      expect(out).not.toContain('not available on the server');
    } finally {
      await mcp.stop().catch(() => {});
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);

  // F6 — a semantic-only request must be rejected honestly (vector search is dark
  // on the server), never silently routed to a dead leg.
  test('search rejects a semantic-only request honestly (F6 semantic dark)', async () => {
    const server = await spawnRustServer();
    const mcpClient = makeClient(server.port, 'mcp-semantic');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      const result = await mcp.callTool('topgun_search', {
        map: `mcp_semantic_${Date.now()}`,
        query: 'anything',
        methods: ['semantic'],
      });

      expect(result.isError).toBe(true);
      expect(text(result)).toMatch(/semantic.*not yet available/i);
      expect(text(result)).toContain('methods: ["fullText"]');
    } finally {
      await mcpClient.close().catch(() => {});
      await server.cleanup().catch(() => {});
    }
  }, 30_000);

  // F7 — topgun_query must NEVER instruct the agent to page with the literal string
  // "undefined". When the server reports hasMore without a usable cursor token (a
  // known getPaginationInfo gap), the pre-fix tool rendered `cursor: "undefined"`.
  // We seed more rows than the page limit and assert the truncation is disclosed but
  // the "undefined" token never appears.
  test('query never renders cursor:"undefined" when more rows exist (F7)', async () => {
    const server = await spawnRustServer();
    const seeder = await createRustTestClient(server.port, {
      userId: 'mcp-seeder',
      roles: ['ADMIN'],
    });
    const mcpClient = makeClient(server.port, 'mcp-pager');
    const mcp = new TopGunMCPServer({ client: mcpClient });

    try {
      await seeder.waitForMessage('AUTH_ACK', 10_000);
      await mcpClient.start();
      await waitForState(mcpClient, SyncState.CONNECTED);

      const mapName = `mcp_page_${Date.now()}`;
      for (let i = 0; i < RECORD_COUNT; i++) {
        seeder.messages.length = 0;
        seeder.send({
          type: 'CLIENT_OP',
          payload: {
            id: `seed-${i}`,
            mapName,
            opType: 'PUT',
            key: `k${i}`,
            record: createLWWRecord({ id: `k${i}`, title: `doc ${i}`, n: i }),
          },
        });
        await seeder.waitForMessage('OP_ACK', 10_000);
      }

      // limit:5 over 25 rows → the server reports more rows exist.
      const result = await callStable(mcp, 'topgun_query', { map: mapName, limit: 5 });
      expect(result.isError).toBeFalsy();
      const out = text(result);
      expect(out).toContain('Found 5 result(s)');
      // Truncation must be disclosed...
      expect(out).toContain('More rows match than were returned');
      // ...but the agent is NEVER handed the string "undefined" as a cursor.
      expect(out).not.toContain('cursor: "undefined"');
      expect(out).not.toContain('"undefined"');
    } finally {
      await mcpClient.close().catch(() => {});
      seeder.close();
      await server.cleanup().catch(() => {});
    }
  }, 60_000);
});
