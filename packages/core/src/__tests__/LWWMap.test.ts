import { HLC } from '../HLC';
import { LWWMap } from '../LWWMap';

describe('LWWMap (Last-Write-Wins CRDT)', () => {
  let hlc: HLC;
  let map: LWWMap<string, string>;

  beforeEach(() => {
    hlc = new HLC('test-node');
    map = new LWWMap(hlc);
  });

  test('should set and get values', () => {
    map.set('key1', 'value1');
    expect(map.get('key1')).toBe('value1');
  });

  test('should handle removal', () => {
    map.set('key1', 'value1');
    map.remove('key1');
    expect(map.get('key1')).toBeUndefined();
    // Record should still exist as tombstone
    const record = map.getRecord('key1');
    expect(record).toBeDefined();
    expect(record?.value).toBeNull();
  });

  test('should resolve conflicts by timestamp (Last Write Wins)', () => {
    const hlc1 = new HLC('node1');
    const hlc2 = new HLC('node2');

    const map1 = new LWWMap<string, string>(hlc1);

    // Advance HLC clocks so they're in a defined state (timestamps not used directly below,
    // but advancing ensures the HLC state is initialized before we create manual records).
    hlc1.now();
    hlc2.now();

    // Record 1: Earlier
    const record1 = {
      value: 'val1',
      timestamp: { millis: 100, counter: 0, nodeId: 'A' },
    };

    // Record 2: Later
    const record2 = {
      value: 'val2',
      timestamp: { millis: 200, counter: 0, nodeId: 'B' },
    };

    // Apply Record 1
    map1.merge('key', record1);
    expect(map1.get('key')).toBe('val1');

    // Apply Record 2 (Newer)
    map1.merge('key', record2);
    expect(map1.get('key')).toBe('val2'); // Should switch to val2

    // Apply Record 1 again (Older)
    map1.merge('key', record1);
    expect(map1.get('key')).toBe('val2'); // Should STAY val2 (LWW)
  });

  test('should resolve ties using nodeId (convergence)', () => {
    const map1 = new LWWMap<string, string>(hlc);

    // Same time, different nodes
    // Node 'B' > Node 'A' lexicographically
    const recordA = {
      value: 'valA',
      timestamp: { millis: 100, counter: 0, nodeId: 'A' },
    };
    const recordB = {
      value: 'valB',
      timestamp: { millis: 100, counter: 0, nodeId: 'B' },
    };

    map1.merge('key', recordA);
    map1.merge('key', recordB);

    expect(map1.get('key')).toBe('valB'); // B wins

    // Try reverse order
    const map2 = new LWWMap<string, string>(hlc);
    map2.merge('key', recordB);
    map2.merge('key', recordA);

    expect(map2.get('key')).toBe('valB'); // B still wins
  });

  test('should prune tombstones correctly', () => {
    // 1. Set item
    map.set('key1', 'value1');

    // 2. Remove item (create tombstone)
    const tombstoneRecord = map.remove('key1');
    expect(map.get('key1')).toBeUndefined();
    expect(map.getRecord('key1')).toBeDefined();
    expect(map.getRecord('key1')?.value).toBeNull();

    // 3. Create a timestamp strictly older than the tombstone
    // tombstone.timestamp is "now".
    const olderTimestamp = {
      millis: tombstoneRecord.timestamp.millis - 1000,
      counter: 0,
      nodeId: 'test-node',
    };

    // 4. Prune with older timestamp -> Should NOT remove
    const prunedKeys1 = map.prune(olderTimestamp);
    expect(prunedKeys1).toEqual([]);
    expect(map.getRecord('key1')).toBeDefined(); // Still there

    // 5. Create a timestamp strictly newer than the tombstone
    const newerTimestamp = {
      millis: tombstoneRecord.timestamp.millis + 1000,
      counter: 0,
      nodeId: 'test-node',
    };

    // 6. Prune with newer timestamp -> Should remove
    const prunedKeys2 = map.prune(newerTimestamp);
    expect(prunedKeys2).toEqual(['key1']);
    expect(map.getRecord('key1')).toBeUndefined(); // GONE

    // Merkle Tree should be updated (removed from tree)
    // We can check via internal merkleTree state if accessible, or assume it's consistent if subsequent ops work.
    // Let's just check if we can add it back.
    map.set('key1', 'newValue');
    expect(map.get('key1')).toBe('newValue');
  });

  test('should respect TTL options', () => {
    const now = Date.now();
    jest.spyOn(Date, 'now').mockImplementation(() => now);

    // 1. Set with TTL (100ms)
    map.set('tempKey', 'tempVal', 100);
    expect(map.get('tempKey')).toBe('tempVal');

    // 2. Advance time by 50ms (Not expired)
    jest.spyOn(Date, 'now').mockImplementation(() => now + 50);
    expect(map.get('tempKey')).toBe('tempVal');

    // 3. Advance time by 150ms (Expired)
    jest.spyOn(Date, 'now').mockImplementation(() => now + 150);
    expect(map.get('tempKey')).toBeUndefined();

    // Restore
    jest.restoreAllMocks();
  });
});

describe('LWWMap.adoptServerEcho (optimistic re-stamp reconciliation, F8)', () => {
  // Same key + value under two timestamps: the client's optimistic HLC (which
  // briefly outran the server) and the server's authoritative arrival-order HLC.
  const key = 'doc';
  const value = { title: 'hello', n: 1 };
  const clientStamp = { millis: 2000, counter: 0, nodeId: 'client' };
  const serverStamp = { millis: 1000, counter: 0, nodeId: 'server' };

  function clientMapWithOptimisticWrite() {
    const map = new LWWMap<string, typeof value>(new HLC('client'));
    // Optimistic local write carrying the (ahead-of-server) client timestamp.
    map.merge(key, { value, timestamp: clientStamp });
    return map;
  }

  function serverMerkleRoot() {
    const server = new LWWMap<string, typeof value>(new HLC('server'));
    server.merge(key, { value, timestamp: serverStamp });
    return server.getMerkleTree().getRootHash();
  }

  test('rejected echo of our own write diverges the Merkle root (the F8 bug)', () => {
    // Negative control: without adoption, the old unconditional-merge behavior
    // leaves the client on the client timestamp while the server is on the
    // server timestamp — the Merkle roots disagree even though the value is
    // identical, which is exactly the perpetual bucket-re-request bug.
    const client = clientMapWithOptimisticWrite();
    const accepted = client.merge(key, { value, timestamp: serverStamp });
    expect(accepted).toBe(false); // server ts < client ts → rejected
    expect(client.getMerkleTree().getRootHash()).not.toBe(serverMerkleRoot());
  });

  test('adopting the rejected echo converges the Merkle root with the server', () => {
    const client = clientMapWithOptimisticWrite();
    const accepted = client.merge(key, { value, timestamp: serverStamp });
    expect(accepted).toBe(false);

    const adopted = client.adoptServerEcho(key, { value, timestamp: serverStamp });
    expect(adopted).toBe(true);

    // Memory now carries the server's authoritative timestamp...
    expect(client.getRecord(key)?.timestamp).toEqual(serverStamp);
    // ...and the value is unchanged...
    expect(client.get(key)).toEqual(value);
    // ...so the Merkle root matches the server's after a single round-trip.
    expect(client.getMerkleTree().getRootHash()).toBe(serverMerkleRoot());
  });

  test('does NOT adopt when a newer local write supersedes the echo (no data loss)', () => {
    const client = clientMapWithOptimisticWrite();
    // A second, newer local write changes the value before the first echo lands.
    const newerValue = { title: 'hello', n: 2 };
    const newerStamp = { millis: 3000, counter: 0, nodeId: 'client' };
    client.merge(key, { value: newerValue, timestamp: newerStamp });

    // The stale echo of the FIRST write arrives and is rejected by LWW...
    expect(client.merge(key, { value, timestamp: serverStamp })).toBe(false);
    // ...and must not be adopted, because its value differs from the live one.
    expect(client.adoptServerEcho(key, { value, timestamp: serverStamp })).toBe(false);

    // The newer local write is preserved intact.
    expect(client.get(key)).toEqual(newerValue);
    expect(client.getRecord(key)?.timestamp).toEqual(newerStamp);
  });

  test('returns false when there is no local record for the key', () => {
    const client = new LWWMap<string, typeof value>(new HLC('client'));
    expect(client.adoptServerEcho(key, { value, timestamp: serverStamp })).toBe(false);
  });

  test('reconciles a rejected tombstone echo (REMOVE) by adopting the server ts', () => {
    const client = new LWWMap<string, typeof value>(new HLC('client'));
    // Local optimistic delete at an ahead-of-server timestamp.
    client.merge(key, { value: null, timestamp: clientStamp });
    expect(client.merge(key, { value: null, timestamp: serverStamp })).toBe(false);
    expect(client.adoptServerEcho(key, { value: null, timestamp: serverStamp })).toBe(true);
    expect(client.getRecord(key)?.timestamp).toEqual(serverStamp);
  });
});
