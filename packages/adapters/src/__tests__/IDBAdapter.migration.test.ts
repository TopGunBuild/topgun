import { IDBAdapter } from '../IDBAdapter';

// SPEC-343 AC7 — the forbid decision adds no destructive key migration; an
// existing (colon-free) store must simply re-open and restore every prior record
// with no data loss. This exercises a pre-populated adapter across a close/reopen
// cycle over the SAME database (the additive schema-version-2 upgrade path).
describe('IDBAdapter migration is non-destructive (SPEC-343 AC7)', () => {
  let dbCounter = 0;
  const getUniqueDbName = () => `migration_db_${Date.now()}_${dbCounter++}`;

  it('an existing colon-free store opens and restores all prior records after reopen', async () => {
    const dbName = getUniqueDbName();

    // 1. Pre-populate a store: KV records (LWW + OR shapes), meta, and a pending op —
    //    all under colon-free map names (the only names creatable post-forbid).
    const first = new IDBAdapter();
    await first.initialize(dbName);
    await first.waitForReady();

    await first.put('users:alice', { value: { name: 'Alice' }, hlc: '1-0-n1' });
    await first.put('tags:post:123', [{ value: 'x', tag: 't1', hlc: '2-0-n1' }]);
    await first.setMeta('__sys__:tags:tombstones', ['t0']);
    await first.setMeta('lastSyncTimestamp', 12345);
    const opId = await first.appendOpLog({
      key: 'alice',
      op: 'PUT',
      value: { name: 'Alice' },
      synced: 0,
      mapName: 'users',
    });
    expect(opId).toBeGreaterThan(0);

    await first.close();

    // 2. Reopen a fresh adapter over the SAME database — re-runs the additive
    //    version-2 upgrade (must not drop/rewrite kv_store or meta_store).
    const reopened = new IDBAdapter();
    await reopened.initialize(dbName);
    await reopened.waitForReady();
    try {
      // 3. Every prior record survives — no data loss.
      expect(await reopened.get('users:alice')).toEqual({
        value: { name: 'Alice' },
        hlc: '1-0-n1',
      });
      // A colon-in-KEY record round-trips unchanged (keys were never restricted).
      expect(await reopened.get('tags:post:123')).toEqual([
        { value: 'x', tag: 't1', hlc: '2-0-n1' },
      ]);
      expect(await reopened.getMeta('__sys__:tags:tombstones')).toEqual(['t0']);
      expect(await reopened.getMeta('lastSyncTimestamp')).toBe(12345);

      const keys = (await reopened.getAllKeys()).sort();
      expect(keys).toEqual(['tags:post:123', 'users:alice']);

      const pending = await reopened.getPendingOps();
      expect(pending.map((p) => p.key)).toEqual(['alice']);
    } finally {
      await reopened.close();
    }
  });
});
