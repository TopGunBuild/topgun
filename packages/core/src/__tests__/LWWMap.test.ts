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
    
    // Create two concurrent updates
    const ts1 = hlc1.now();
    const ts2 = hlc2.now();
    
    // Artificially ensure ts2 > ts1
    // In HLC, we can just update one HLC with the other to sync, 
    // but here we want to simulate concurrent disconnected writes.
    // TS2 should generally be "newer" if created later, but to be sure:
    // let's rely on HLC.compare logic. 
    // Actually, let's just create records manually to control timestamps.
    
    // Record 1: Earlier
    const record1 = { value: 'val1', timestamp: { millis: 100, counter: 0, nodeId: 'A' } };
    
    // Record 2: Later
    const record2 = { value: 'val2', timestamp: { millis: 200, counter: 0, nodeId: 'B' } };

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
    const recordA = { value: 'valA', timestamp: { millis: 100, counter: 0, nodeId: 'A' } };
    const recordB = { value: 'valB', timestamp: { millis: 100, counter: 0, nodeId: 'B' } };

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
      nodeId: 'test-node' 
    };

    // 4. Prune with older timestamp -> Should NOT remove
    const prunedKeys1 = map.prune(olderTimestamp);
    expect(prunedKeys1).toEqual([]);
    expect(map.getRecord('key1')).toBeDefined(); // Still there

    // 5. Create a timestamp strictly newer than the tombstone
    const newerTimestamp = { 
        millis: tombstoneRecord.timestamp.millis + 1000, 
        counter: 0, 
        nodeId: 'test-node' 
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
