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
  // Op ids whose durable delete must FAIL (to exercise the fail-closed abort path).
  public failDeleteOpIds = new Set<number>();
  // Storage keys whose durable `remove` must FAIL (to exercise the materialized-key
  // fail-closed abort path).
  public failRemoveKeys = new Set<string>();
  // Meta keys whose durable `setMeta` must FAIL (to exercise the fail-closed abort
  // of a meta write that has to land before in-memory state is discarded).
  public failSetMetaKeys = new Set<string>();

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
    if (this.failRemoveKeys.has(key)) {
      throw new Error(`simulated durable remove failure for key ${key}`);
    }
    this.kv.delete(key);
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  async getMeta(key: string): Promise<any> {
    return this.meta.get(key);
  }
  async setMeta(key: string, value: unknown): Promise<void> {
    if (this.failSetMetaKeys.has(key)) {
      throw new Error(`simulated durable setMeta failure for key ${key}`);
    }
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
    if (this.failDeleteOpIds.has(id)) {
      throw new Error(`simulated durable deleteOp failure for id ${id}`);
    }
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

    await eng.replaceOrMapFromSnapshot('tags', undefined);

    const remaining = eng.opLog.map((o: OpLogEntry) => `${o.mapName}:${o.id}`);
    expect(remaining).toEqual(['other:3']);
  });

  test('a subsumed op whose durable delete FAILS aborts the resync (op survives in BOTH memory and disk — never dropped-from-memory-but-kept-on-disk)', async () => {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    await new Promise((r) => setTimeout(r, 25));

    const hlc = new HLC('n1');
    const preTs = hlc.now();
    const boundary = hlc.now(); // strictly after preTs

    const map = new ORMap<string, string>(hlc);
    engine.registerMap('tags', map);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- test seeds the private opLog
    const eng = engine as any;
    eng.opLog.push(orOp('1', 'kPre', '10:0:n1', preTs));
    adapter.ops.push({ id: 1 } as StorageOpLogEntry);
    // Force the durable delete of the subsumed op to fail.
    adapter.failDeleteOpIds.add(1);

    await expect(eng.replaceOrMapFromSnapshot('tags', boundary)).rejects.toThrow();

    // Fail-closed: the subsumed op must NOT have been dropped from memory while
    // surviving on disk — it stays recoverable in BOTH layers so the retried resync
    // re-attempts its durable delete rather than resurrecting it on the next restart.
    const remainingIds = eng.opLog.map((o: OpLogEntry) => o.id);
    expect(remainingIds).toContain('1');
    expect(adapter.ops.some((o) => o.id === 1)).toBe(true);
    expect(adapter.deletedOpIds).not.toContain(1);
  });

  test('a materialized-key durable remove FAILURE aborts the resync (in-memory state + tombstones intact — no orphan-on-disk after clearing the tombstone)', async () => {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    await new Promise((r) => setTimeout(r, 25));

    const hlc = new HLC('n1');
    const map = new ORMap<string, string>(hlc);
    map.add('kOrphan', 'stale'); // local materialized state to discard
    engine.registerMap('tags', map);

    // Force the durable removal of the materialized record to fail.
    adapter.failRemoveKeys.add('tags:kOrphan');

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- test reaches the private REPLACE
    const eng = engine as any;
    await expect(eng.replaceOrMapFromSnapshot('tags', hlc.now())).rejects.toThrow();

    // Fail-closed: the in-memory map must NOT have been cleared while the durable
    // record survives — clearing first would strand an un-removable orphan on disk
    // (with its tombstone cleared) that re-materializes + re-pushes after restart →
    // resurrection. Aborting BEFORE the clear keeps memory and disk consistent so
    // the retried resync re-discovers the key and re-attempts its removal.
    expect(map.allKeys()).toContain('kOrphan');
  });

  test('a subsumed op with a non-numeric id is a HARD failure (abort), not a silent skip that leaves it durably retained', async () => {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    await new Promise((r) => setTimeout(r, 25));

    const hlc = new HLC('n1');
    const preTs = hlc.now();
    const boundary = hlc.now();

    const map = new ORMap<string, string>(hlc);
    engine.registerMap('tags', map);

    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- test seeds the private opLog
    const eng = engine as any;
    eng.opLog.push(orOp('not-a-number', 'kPre', '10:0:n1', preTs));

    await expect(eng.replaceOrMapFromSnapshot('tags', boundary)).rejects.toThrow();

    // The un-addressable op stays in memory (not silently spliced) — it is never
    // dropped-from-memory-but-kept-on-disk.
    const remainingIds = eng.opLog.map((o: OpLogEntry) => o.id);
    expect(remainingIds).toContain('not-a-number');
  });
});

/**
 * Per-key tombstone attribution across an authoritative REPLACE resync.
 *
 * REPLACE wipes the map, so every key's attribution goes with it, in memory and
 * on disk. What survives is a local remove the server has not acknowledged and
 * that REPLACE retains: its tag is not stored with the map but read from the op
 * log each time a response rebuilds a key's attribution, so it comes back as
 * soon as the snapshot pull reaches that key.
 */
describe('SyncEngine REPLACE resync — per-key tombstone attribution', () => {
  const KEY_TOMBSTONES_META = '__sys__:tags:keyTombstones';

  async function makeEngine() {
    const adapter = new MemoryAdapter();
    const engine = new SyncEngine({
      nodeId: 'n1',
      connectionProvider: new NullConnectionProvider(),
      storageAdapter: adapter,
    });
    // Let the constructor's async op-log load settle so it does not clear the
    // ops recorded below.
    await new Promise((r) => setTimeout(r, 25));
    const map = new ORMap<string, string>(engine.getHLC());
    engine.registerMap('tags', map);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the test drives the engine's private sync handler and REPLACE directly
    const eng = engine as any;
    return { adapter, engine, eng, map };
  }

  /**
   * Key K holds the value `removed-value` under tag tR and the client removes
   * it; the remove is committed to the op log but not acknowledged. The server
   * then forces a full resync whose snapshot boundary PRECEDES that remove, so
   * REPLACE retains the op while wiping the map (items, map-wide tombstones and
   * attribution alike).
   */
  async function replaceRetainingUnackedRemove() {
    const ctx = await makeEngine();
    const { engine, eng, map } = ctx;
    const hlc = engine.getHLC();

    const boundary = hlc.now();
    const removedRecord = map.add('K', 'removed-value');
    const [tR] = map.remove('K', 'removed-value');
    expect(tR).toBe(removedRecord.tag);
    // Await the commit: the op is in the op log before any response is injected.
    const opId = await engine.recordOperation('tags', 'OR_REMOVE', 'K', {
      orTag: tR,
      timestamp: hlc.now(),
    });
    await engine.persistORMapKeyTombstones('tags');
    expect(ctx.adapter.meta.get(KEY_TOMBSTONES_META)).toEqual([['K', [tR]]]);

    await eng.orMapSyncHandler.handleORMapSyncRespRoot({
      mapName: 'tags',
      rootHash: 12345,
      fullResync: true,
      timestamp: boundary,
    });

    const retained = eng.opLog.map((o: OpLogEntry) => o.id);
    expect(retained).toEqual([opId]);
    return { ...ctx, tR, removedRecord };
  }

  test('after REPLACE and before the snapshot pull, no attribution is left in memory or on disk', async () => {
    const { adapter, engine, map } = await replaceRetainingUnackedRemove();

    expect(adapter.meta.get(KEY_TOMBSTONES_META)).toEqual([]);
    expect(map.getKeyTombstones('K').size).toBe(0);
    expect(map.getSnapshot().keyTombstones.size).toBe(0);
    engine.close();
  });

  test('the snapshot leaf for the key yields the server set plus the retained remove tag', async () => {
    const { adapter, engine, eng, map, tR } = await replaceRetainingUnackedRemove();

    await eng.orMapSyncHandler.handleORMapSyncRespLeaf({
      mapName: 'tags',
      path: '',
      entries: [{ key: 'K', records: [], tombstones: ['server-tag-1', 'server-tag-2'] }],
    });

    expect([...map.getKeyTombstones('K')].sort()).toEqual(
      ['server-tag-1', 'server-tag-2', tR].sort(),
    );
    const persisted = adapter.meta.get(KEY_TOMBSTONES_META) as Array<[string, string[]]>;
    expect(persisted).toHaveLength(1);
    expect(persisted[0][0]).toBe('K');
    expect([...persisted[0][1]].sort()).toEqual(['server-tag-1', 'server-tag-2', tR].sort());
    engine.close();
  });

  test('the snapshot leaf re-delivering the removed record never renders it: attribution is applied before the merge', async () => {
    const { engine, eng, map, tR, removedRecord } = await replaceRetainingUnackedRemove();

    // Precondition: REPLACE wiped the map-wide tombstone set, so nothing but the
    // attribution step can keep the record out. Without this the test could not
    // tell the two orders apart, because a tag already suppressed map-wide is
    // skipped by the merge in either order.
    expect(map.getTombstones()).not.toContain(tR);

    const rendered: Array<Array<[string, string[]]>> = [];
    map.subscribe((entries) => rendered.push(entries));

    await eng.orMapSyncHandler.handleORMapSyncRespLeaf({
      mapName: 'tags',
      path: '',
      entries: [{ key: 'K', records: [removedRecord], tombstones: [] }],
    });

    const renderedRemovedValue = rendered
      .flat()
      .some(([key, values]) => key === 'K' && values.includes('removed-value'));
    expect(renderedRemovedValue).toBe(false);
    expect(map.getTombstones()).toContain(tR);
    expect(map.getKeyTombstones('K').has(tR)).toBe(true);
    expect(map.get('K')).not.toContain('removed-value');
    engine.close();
  });

  test('a full-resync REPLACE removes the persisted attribution', async () => {
    const { adapter, engine, eng, map } = await makeEngine();
    map.add('K', 'kept');
    map.add('K', 'gone');
    map.remove('K', 'gone');
    await engine.persistORMapKeyTombstones('tags');
    expect((adapter.meta.get(KEY_TOMBSTONES_META) as unknown[]).length).toBe(1);

    await eng.replaceOrMapFromSnapshot('tags', undefined);

    expect(adapter.meta.get(KEY_TOMBSTONES_META)).toEqual([]);
    expect(map.getKeyTombstones('K').size).toBe(0);
    engine.close();
  });

  test('a FAILED attribution reset aborts the resync before the map is cleared (memory and disk both keep the old attribution)', async () => {
    const { adapter, engine, eng, map } = await makeEngine();
    map.add('K', 'kept');
    map.add('K', 'gone');
    const [tag] = map.remove('K', 'gone');
    await engine.persistORMapKeyTombstones('tags');
    const persistedBefore = adapter.meta.get(KEY_TOMBSTONES_META);
    expect(persistedBefore).toEqual([['K', [tag]]]);

    const clear = jest.spyOn(map, 'clear');
    adapter.failSetMetaKeys.add(KEY_TOMBSTONES_META);

    await expect(eng.replaceOrMapFromSnapshot('tags', engine.getHLC().now())).rejects.toThrow(
      'simulated durable setMeta failure',
    );

    // Fail-closed: the durable reset comes first, so its failure leaves the
    // in-memory attribution untouched instead of clearing it while disk still
    // claims the old one.
    expect(clear).not.toHaveBeenCalled();
    expect(map.get('K')).toEqual(['kept']);
    expect([...map.getKeyTombstones('K')]).toEqual([tag]);
    expect(adapter.meta.get(KEY_TOMBSTONES_META)).toEqual(persistedBefore);
    engine.close();
  });
});
