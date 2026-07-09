import { HLC, ORMap } from '@topgunbuild/core';
import type { ORMapRecord, Timestamp } from '@topgunbuild/core';
import { SyncEngine } from '../SyncEngine';
import type { OpLogEntry } from '../SyncEngine';
import { NullConnectionProvider } from '../connection/NullConnectionProvider';
import { IStorageAdapter, OpLogEntry as StorageOpLogEntry } from '../IStorageAdapter';

/**
 * SPEC-342c AC15 — on an authoritative REPLACE resync the client discards pending
 * oplog OR ops whose HLC PRECEDES the snapshot boundary (subsumed by the server
 * snapshot; replaying them would resurrect a pruned tag via the write path) while
 * KEEPING ops at-or-after the boundary (re-driven through the normal gated push).
 * The comparable is the pending op's own HLC vs the snapshot's HLC boundary.
 */
class MemoryAdapter implements IStorageAdapter {
  public kv = new Map<string, unknown>();
  public meta = new Map<string, unknown>();
  public ops: StorageOpLogEntry[] = [];
  public deletedOpIds: number[] = [];

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
  async removeMeta(key: string): Promise<void> {
    this.meta.delete(key);
  }
  async batchPut(entries: Map<string, unknown>): Promise<void> {
    for (const [k, v] of entries) this.kv.set(k, v);
  }
  async appendOpLog(entry: Omit<StorageOpLogEntry, 'id'>): Promise<number> {
    const id = this.ops.length + 1;
    this.ops.push({ ...entry, id } as StorageOpLogEntry);
    return id;
  }
  async getPendingOps(): Promise<StorageOpLogEntry[]> {
    return this.ops;
  }
  async markOpsSynced(lastId: number): Promise<void> {
    this.ops = this.ops.filter((o) => (o.id ?? 0) > lastId);
  }
  async deleteOp(id: number): Promise<void> {
    this.deletedOpIds.push(id);
    this.ops = this.ops.filter((o) => o.id !== id);
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  async commitWrite(mutations: any[], op: Omit<StorageOpLogEntry, 'id'>): Promise<number> {
    for (const m of mutations) {
      const target = m.store === 'meta' ? this.meta : this.kv;
      if (m.type === 'remove') target.delete(m.key);
      else target.set(m.key, m.value);
    }
    return this.appendOpLog(op);
  }
  async getAllKeys(): Promise<string[]> {
    return [...this.kv.keys()];
  }
  async getAllMetaKeys(): Promise<string[]> {
    return [...this.meta.keys()];
  }
  async clear(): Promise<void> {
    this.kv.clear();
    this.meta.clear();
    this.ops = [];
  }
}

function orOp(id: string, key: string, tag: string, ts: Timestamp): OpLogEntry {
  const orRecord: ORMapRecord<string> = { value: key, tag, timestamp: ts };
  return {
    id,
    mapName: 'tags',
    opType: 'OR_ADD',
    key,
    orRecord,
    timestamp: ts,
    synced: false,
  };
}

describe('SyncEngine REPLACE resync — pending-oplog HLC discard (AC15)', () => {
  test('discards pre-boundary OR ops, keeps at-or-after ops', async () => {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });

    // The constructor kicks off an async loadOpLog() that resets opLog; let it
    // settle before seeding controlled pending ops so they are not cleared.
    await new Promise((r) => setTimeout(r, 25));

    const hlc = new HLC('n1');
    const preTs = hlc.now();
    const boundary = hlc.now(); // strictly after preTs
    const postTs = hlc.now(); // strictly after boundary

    const map = new ORMap<string, string>(hlc);
    map.add('kPre', 'stale'); // local materialized state to discard
    engine.registerMap('tags', map);

    // Inject two pending OR ops: one before the snapshot boundary (subsumed) and
    // one at-or-after it (must survive to be re-driven through the gated push).
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- test reaches into the private opLog to seed controlled-HLC pending ops
    const eng = engine as any;
    eng.opLog.push(orOp('1', 'kPre', '10:0:n1', preTs));
    eng.opLog.push(orOp('2', 'kPost', '30:0:n1', postTs));
    adapter.ops.push({ id: 1 } as StorageOpLogEntry, { id: 2 } as StorageOpLogEntry);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- exercise the private REPLACE path directly
    await eng.replaceOrMapFromSnapshot('tags', boundary);

    const remainingIds = eng.opLog.map((o: OpLogEntry) => o.id);
    expect(remainingIds).toEqual(['2']); // pre-boundary op discarded, at-or-after kept
    expect(adapter.deletedOpIds).toEqual([1]); // durably removed too
    // Local materialized state was discarded (REPLACE), pending snapshot pull.
    expect(map.allKeys()).toEqual([]);
  });

  test('with no snapshot boundary, all pending OR ops for the map are discarded (conservative REPLACE)', async () => {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    await new Promise((r) => setTimeout(r, 25));
    const hlc = new HLC('n1');
    const map = new ORMap<string, string>(hlc);
    engine.registerMap('tags', map);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const eng = engine as any;
    eng.opLog.push(orOp('1', 'a', '1:0:n1', hlc.now()));
    eng.opLog.push(orOp('2', 'b', '2:0:n1', hlc.now()));
    // A pending op for ANOTHER map must be untouched.
    eng.opLog.push({ ...orOp('3', 'c', '3:0:n1', hlc.now()), mapName: 'other' });

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    await eng.replaceOrMapFromSnapshot('tags', undefined);

    const remaining = eng.opLog.map((o: OpLogEntry) => `${o.mapName}:${o.id}`);
    expect(remaining).toEqual(['other:3']);
  });
});
