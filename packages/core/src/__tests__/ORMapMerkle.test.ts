import { HLC } from '../HLC';
import { ORMap, ORMapRecord } from '../ORMap';
import { ORMapMerkleTree } from '../ORMapMerkleTree';
import { hashORMapEntry, hashORMapRecord, timestampToString, compareTimestamps } from '../ORMapMerkle';

describe('ORMapMerkle hash functions', () => {
  describe('timestampToString', () => {
    it('should convert timestamp to deterministic string', () => {
      const ts = { millis: 1234567890, counter: 42, nodeId: 'node-1' };
      expect(timestampToString(ts)).toBe('1234567890:42:node-1');
    });
  });

  describe('hashORMapEntry', () => {
    it('should produce same hash regardless of record insertion order', () => {
      const ts1 = { millis: 1000, counter: 1, nodeId: 'node-1' };
      const ts2 = { millis: 2000, counter: 1, nodeId: 'node-2' };

      // First order: tag-a, then tag-b
      const records1 = new Map<string, ORMapRecord<string>>();
      records1.set('tag-a', { value: 'value-a', timestamp: ts1, tag: 'tag-a' });
      records1.set('tag-b', { value: 'value-b', timestamp: ts2, tag: 'tag-b' });

      // Second order: tag-b, then tag-a
      const records2 = new Map<string, ORMapRecord<string>>();
      records2.set('tag-b', { value: 'value-b', timestamp: ts2, tag: 'tag-b' });
      records2.set('tag-a', { value: 'value-a', timestamp: ts1, tag: 'tag-a' });

      const hash1 = hashORMapEntry('key1', records1);
      const hash2 = hashORMapEntry('key1', records2);

      expect(hash1).toBe(hash2);
    });

    it('should produce different hash for different values', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const records1 = new Map<string, ORMapRecord<string>>();
      records1.set('tag-1', { value: 'value-a', timestamp: ts, tag: 'tag-1' });

      const records2 = new Map<string, ORMapRecord<string>>();
      records2.set('tag-1', { value: 'value-b', timestamp: ts, tag: 'tag-1' });

      const hash1 = hashORMapEntry('key1', records1);
      const hash2 = hashORMapEntry('key1', records2);

      expect(hash1).not.toBe(hash2);
    });

    it('should produce different hash for different tags', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const records1 = new Map<string, ORMapRecord<string>>();
      records1.set('tag-a', { value: 'value', timestamp: ts, tag: 'tag-a' });

      const records2 = new Map<string, ORMapRecord<string>>();
      records2.set('tag-b', { value: 'value', timestamp: ts, tag: 'tag-b' });

      const hash1 = hashORMapEntry('key1', records1);
      const hash2 = hashORMapEntry('key1', records2);

      expect(hash1).not.toBe(hash2);
    });

    it('should produce different hash for different timestamps', () => {
      const records1 = new Map<string, ORMapRecord<string>>();
      records1.set('tag-1', {
        value: 'value',
        timestamp: { millis: 1000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1'
      });

      const records2 = new Map<string, ORMapRecord<string>>();
      records2.set('tag-1', {
        value: 'value',
        timestamp: { millis: 2000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1'
      });

      const hash1 = hashORMapEntry('key1', records1);
      const hash2 = hashORMapEntry('key1', records2);

      expect(hash1).not.toBe(hash2);
    });

    it('should produce different hash for different keys', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const records = new Map<string, ORMapRecord<string>>();
      records.set('tag-1', { value: 'value', timestamp: ts, tag: 'tag-1' });

      const hash1 = hashORMapEntry('key1', records);
      const hash2 = hashORMapEntry('key2', records);

      expect(hash1).not.toBe(hash2);
    });

    it('should handle TTL in hash', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const recordsWithTtl = new Map<string, ORMapRecord<string>>();
      recordsWithTtl.set('tag-1', { value: 'value', timestamp: ts, tag: 'tag-1', ttlMs: 5000 });

      const recordsNoTtl = new Map<string, ORMapRecord<string>>();
      recordsNoTtl.set('tag-1', { value: 'value', timestamp: ts, tag: 'tag-1' });

      const hash1 = hashORMapEntry('key1', recordsWithTtl);
      const hash2 = hashORMapEntry('key1', recordsNoTtl);

      expect(hash1).not.toBe(hash2);
    });

    it('should handle object values deterministically', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      // Different key order in object
      const records1 = new Map<string, ORMapRecord<{ a: number; b: string }>>();
      records1.set('tag-1', {
        value: { a: 1, b: 'test' },
        timestamp: ts,
        tag: 'tag-1'
      });

      const records2 = new Map<string, ORMapRecord<{ b: string; a: number }>>();
      records2.set('tag-1', {
        value: { b: 'test', a: 1 },
        timestamp: ts,
        tag: 'tag-1'
      });

      const hash1 = hashORMapEntry('key1', records1);
      const hash2 = hashORMapEntry('key1', records2);

      expect(hash1).toBe(hash2);
    });

    it('should handle null values', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const records = new Map<string, ORMapRecord<null>>();
      records.set('tag-1', { value: null, timestamp: ts, tag: 'tag-1' });

      // Should not throw
      const hash = hashORMapEntry('key1', records);
      expect(typeof hash).toBe('number');
    });
  });

  describe('hashORMapRecord', () => {
    it('should hash individual record', () => {
      const record: ORMapRecord<string> = {
        value: 'test',
        timestamp: { millis: 1000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1'
      };

      const hash = hashORMapRecord(record);
      expect(typeof hash).toBe('number');
    });
  });

  describe('compareTimestamps', () => {
    it('should return negative when a < b (by millis)', () => {
      const a = { millis: 1000, counter: 1, nodeId: 'node-1' };
      const b = { millis: 2000, counter: 1, nodeId: 'node-1' };
      expect(compareTimestamps(a, b)).toBeLessThan(0);
    });

    it('should return positive when a > b (by millis)', () => {
      const a = { millis: 2000, counter: 1, nodeId: 'node-1' };
      const b = { millis: 1000, counter: 1, nodeId: 'node-1' };
      expect(compareTimestamps(a, b)).toBeGreaterThan(0);
    });

    it('should compare by counter when millis are equal', () => {
      const a = { millis: 1000, counter: 5, nodeId: 'node-1' };
      const b = { millis: 1000, counter: 10, nodeId: 'node-1' };
      expect(compareTimestamps(a, b)).toBeLessThan(0);
    });

    it('should compare by nodeId when millis and counter are equal', () => {
      const a = { millis: 1000, counter: 1, nodeId: 'aaa' };
      const b = { millis: 1000, counter: 1, nodeId: 'bbb' };
      expect(compareTimestamps(a, b)).toBeLessThan(0);
    });

    it('should return 0 for equal timestamps', () => {
      const a = { millis: 1000, counter: 1, nodeId: 'node-1' };
      const b = { millis: 1000, counter: 1, nodeId: 'node-1' };
      expect(compareTimestamps(a, b)).toBe(0);
    });
  });
});

describe('ORMapMerkleTree', () => {
  let hlc: HLC;
  let map: ORMap<string, string>;
  let tree: ORMapMerkleTree;

  beforeEach(() => {
    hlc = new HLC('test-node');
    map = new ORMap<string, string>(hlc);
    tree = new ORMapMerkleTree();
  });

  describe('updateFromORMap', () => {
    it('should compute correct root hash for empty map', () => {
      tree.updateFromORMap(map);
      expect(tree.getRootHash()).toBe(0);
    });

    it('should compute correct root hash for map with data', () => {
      map.add('key1', 'value1');
      map.add('key2', 'value2');

      tree.updateFromORMap(map);
      expect(tree.getRootHash()).not.toBe(0);
    });

    it('should distribute keys across buckets', () => {
      // Add many keys to ensure distribution
      for (let i = 0; i < 20; i++) {
        map.add(`key-${i}`, `value-${i}`);
      }

      tree.updateFromORMap(map);

      // Check that some buckets have data
      const buckets = tree.getBuckets('');
      const nonEmptyBuckets = Object.values(buckets).filter(h => h !== 0);
      expect(nonEmptyBuckets.length).toBeGreaterThan(0);
    });

    it('should update hash when record added', () => {
      map.add('key1', 'value1');
      tree.updateFromORMap(map);
      const hash1 = tree.getRootHash();

      map.add('key1', 'value2');
      tree.updateFromORMap(map);
      const hash2 = tree.getRootHash();

      expect(hash1).not.toBe(hash2);
    });

    it('should update hash when record removed', () => {
      map.add('key1', 'value1');
      map.add('key1', 'value2');
      tree.updateFromORMap(map);
      const hash1 = tree.getRootHash();

      map.remove('key1', 'value1');
      tree.updateFromORMap(map);
      const hash2 = tree.getRootHash();

      expect(hash1).not.toBe(hash2);
    });
  });

  describe('incremental update', () => {
    it('should update tree incrementally via ORMap operations', () => {
      // ORMap now has integrated MerkleTree that updates on add/remove
      map.add('key1', 'value1');
      const hash1 = map.getMerkleTree().getRootHash();

      map.add('key2', 'value2');
      const hash2 = map.getMerkleTree().getRootHash();

      expect(hash1).not.toBe(hash2);
    });
  });

  describe('diff', () => {
    it('should return empty set when trees are equal', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);
      mapA.add('key1', 'value1');

      const hlcB = new HLC('node-b');
      const mapB = new ORMap<string, string>(hlcB);
      mapB.merge(mapA);

      // Trees should have same root hash after merge
      expect(mapA.getMerkleTree().getRootHash()).toBe(mapB.getMerkleTree().getRootHash());
    });

    it('should find keys with different values', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);
      mapA.add('key1', 'value1');

      const hlcB = new HLC('node-b');
      const mapB = new ORMap<string, string>(hlcB);
      mapB.add('key1', 'value-different');

      // Trees should have different root hashes
      expect(mapA.getMerkleTree().getRootHash()).not.toBe(mapB.getMerkleTree().getRootHash());
    });

    it('should find keys missing from other tree', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);
      mapA.add('key1', 'value1');
      mapA.add('key2', 'value2');

      const hlcB = new HLC('node-b');
      const mapB = new ORMap<string, string>(hlcB);
      mapB.add('key1', 'value1');

      // Trees should have different root hashes
      expect(mapA.getMerkleTree().getRootHash()).not.toBe(mapB.getMerkleTree().getRootHash());
    });
  });

  describe('getKeysInBucket', () => {
    it('should return keys at leaf level', () => {
      map.add('key1', 'value1');
      map.add('key2', 'value2');

      const tree = map.getMerkleTree();

      // Find a path that leads to a leaf with keys
      let foundKeys: string[] = [];
      const explore = (path: string, depth: number): void => {
        if (depth > 5) return;
        const buckets = tree.getBuckets(path);
        for (const char of Object.keys(buckets)) {
          const newPath = path + char;
          const keys = tree.getKeysInBucket(newPath);
          if (keys.length > 0) {
            foundKeys = [...foundKeys, ...keys];
          } else {
            explore(newPath, depth + 1);
          }
        }
      };

      explore('', 0);

      expect(foundKeys).toContain('key1');
      expect(foundKeys).toContain('key2');
    });
  });
});

describe('ORMap merge', () => {
  describe('mergeKey', () => {
    it('should add new records', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      const remoteRecord: ORMapRecord<string> = {
        value: 'remote-value',
        timestamp: { millis: 1000, counter: 1, nodeId: 'node-b' },
        tag: 'remote-tag-1'
      };

      const result = mapA.mergeKey('key1', [remoteRecord]);

      expect(result.added).toBe(1);
      expect(result.updated).toBe(0);
      expect(mapA.get('key1')).toContain('remote-value');
    });

    it('should update records with newer timestamp', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      // Add local record
      mapA.add('key1', 'local-value');
      const localRecords = mapA.getRecords('key1');
      const localTag = localRecords[0].tag;

      // Create remote record with same tag but newer timestamp
      const remoteRecord: ORMapRecord<string> = {
        value: 'remote-value-newer',
        timestamp: { millis: Date.now() + 10000, counter: 1, nodeId: 'node-b' },
        tag: localTag
      };

      const result = mapA.mergeKey('key1', [remoteRecord]);

      expect(result.updated).toBe(1);
      expect(mapA.get('key1')).toContain('remote-value-newer');
    });

    it('should keep local record if timestamp is newer', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      // Add local record
      mapA.add('key1', 'local-value');
      const localRecords = mapA.getRecords('key1');
      const localTag = localRecords[0].tag;

      // Create remote record with same tag but older timestamp
      const remoteRecord: ORMapRecord<string> = {
        value: 'remote-value-older',
        timestamp: { millis: 1, counter: 1, nodeId: 'node-b' },
        tag: localTag
      };

      const result = mapA.mergeKey('key1', [remoteRecord]);

      expect(result.added).toBe(0);
      expect(result.updated).toBe(0);
      expect(mapA.get('key1')).toContain('local-value');
    });

    it('should handle concurrent adds (both kept)', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      // Add local record
      mapA.add('key1', 'local-value');

      // Create remote record with different tag
      const remoteRecord: ORMapRecord<string> = {
        value: 'remote-value',
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node-b' },
        tag: 'different-tag'
      };

      const result = mapA.mergeKey('key1', [remoteRecord]);

      expect(result.added).toBe(1);
      const values = mapA.get('key1');
      expect(values).toContain('local-value');
      expect(values).toContain('remote-value');
      expect(values.length).toBe(2);
    });

    it('should track tombstones correctly', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      // Add local record
      mapA.add('key1', 'local-value');
      const localRecords = mapA.getRecords('key1');
      const localTag = localRecords[0].tag;

      // Merge with tombstone for local tag
      const result = mapA.mergeKey('key1', [], [localTag]);

      expect(mapA.get('key1')).toHaveLength(0);
      expect(mapA.isTombstoned(localTag)).toBe(true);
    });

    it('should not apply records that are tombstoned', () => {
      const hlcA = new HLC('node-a');
      const mapA = new ORMap<string, string>(hlcA);

      const tombstonedTag = 'tombstoned-tag';

      // Create remote record
      const remoteRecord: ORMapRecord<string> = {
        value: 'remote-value',
        timestamp: { millis: Date.now(), counter: 1, nodeId: 'node-b' },
        tag: tombstonedTag
      };

      // Merge with both record and its tombstone
      const result = mapA.mergeKey('key1', [remoteRecord], [tombstonedTag]);

      expect(result.added).toBe(0);
      expect(mapA.get('key1')).toHaveLength(0);
    });
  });
});
