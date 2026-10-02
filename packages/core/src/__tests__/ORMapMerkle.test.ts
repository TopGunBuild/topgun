import { HLC } from '../HLC';
import { ORMap, ORMapRecord } from '../ORMap';
import { ORMapMerkleTree } from '../ORMapMerkleTree';
import { hashORMapLeaf, timestampToString, compareTimestamps } from '../ORMapMerkle';
import { hashString } from '../utils/hash';

/** The trie routes a key by the hex digits of its hash and keeps entries at depth 3. */
function leafPathOf(key: string): string {
  return hashString(key).toString(16).padStart(8, '0').slice(0, 3);
}

/** The leaf an ORMap's own tree stores for `key`, or undefined when the key is absent. */
function storedLeaf<V>(map: ORMap<string, V>, key: string): number | undefined {
  return map.getMerkleTree().getEntryHashes(leafPathOf(key)).get(key);
}

describe('ORMapMerkle hash functions', () => {
  describe('timestampToString', () => {
    it('should convert timestamp to deterministic string', () => {
      const ts = { millis: 1234567890, counter: 42, nodeId: 'node-1' };
      expect(timestampToString(ts)).toBe('1234567890:42:node-1');
    });
  });

  describe('hashORMapLeaf', () => {
    it('should produce same hash regardless of tag and tombstone input order', () => {
      const hash1 = hashORMapLeaf('key1', ['tag-a', 'tag-b'], ['dead-a', 'dead-b']);
      const hash2 = hashORMapLeaf('key1', ['tag-b', 'tag-a'], ['dead-b', 'dead-a']);

      expect(hash1).toBe(hash2);
    });

    it('should be deterministic across calls', () => {
      expect(hashORMapLeaf('key1', ['tag-a'], ['dead-a'])).toBe(
        hashORMapLeaf('key1', ['tag-a'], ['dead-a']),
      );
    });

    it('should hash the documented string (TG-MRK-001)', () => {
      // Written out by hand so a change to the formula cannot pass by changing
      // both sides at once.
      expect(hashORMapLeaf('key1', ['tag-b', 'tag-a'], ['dead-b', 'dead-a'])).toBe(
        hashString('key:key1|tag-a|tag-b#dead-a|dead-b'),
      );
      expect(hashORMapLeaf('key1', ['tag-a'], [])).toBe(hashString('key:key1|tag-a#'));
      expect(hashORMapLeaf('key1', [], ['dead-a'])).toBe(hashString('key:key1|#dead-a'));
    });

    it('should produce different hash for different tags', () => {
      const hash1 = hashORMapLeaf('key1', ['tag-a'], []);
      const hash2 = hashORMapLeaf('key1', ['tag-b'], []);

      expect(hash1).not.toBe(hash2);
    });

    it('should produce different hash for different keys', () => {
      const hash1 = hashORMapLeaf('key1', ['tag-1'], []);
      const hash2 = hashORMapLeaf('key2', ['tag-1'], []);

      expect(hash1).not.toBe(hash2);
    });

    it('should produce different hash for different tombstones', () => {
      const none = hashORMapLeaf('key1', ['tag-1'], []);
      const one = hashORMapLeaf('key1', ['tag-1'], ['dead-a']);
      const other = hashORMapLeaf('key1', ['tag-1'], ['dead-b']);
      const two = hashORMapLeaf('key1', ['tag-1'], ['dead-a', 'dead-b']);

      expect(new Set([none, one, other, two]).size).toBe(4);
    });

    it('should tell a live tag from the same tag as a tombstone', () => {
      expect(hashORMapLeaf('key1', ['tag-1'], [])).not.toBe(hashORMapLeaf('key1', [], ['tag-1']));
    });

    it('should treat its inputs as sets', () => {
      expect(hashORMapLeaf('key1', ['tag-a', 'tag-a', 'tag-b'], ['dead-a', 'dead-a'])).toBe(
        hashORMapLeaf('key1', ['tag-a', 'tag-b'], ['dead-a']),
      );
      expect(hashORMapLeaf('key1', new Set(['tag-a', 'tag-b']), new Set(['dead-a']))).toBe(
        hashORMapLeaf('key1', ['tag-a', 'tag-b'], ['dead-a']),
      );
      expect(hashORMapLeaf('key1', new Map([['tag-a', 1]]).keys(), [])).toBe(
        hashORMapLeaf('key1', ['tag-a'], []),
      );
    });

    it('should sort tags by code point, not by UTF-16 code unit', () => {
      // U+10000 is stored as the surrogate pair D800 DC00, which a code-unit
      // sort puts before U+FF61. By code point it comes after.
      const astral = '\u{10000}';
      const highBmp = '\uff61';

      expect([astral, highBmp].sort()).toEqual([astral, highBmp]);
      expect(hashORMapLeaf('key1', [astral, highBmp, 'a'], [astral, highBmp])).toBe(
        hashString(`key:key1|a|${highBmp}|${astral}#${highBmp}|${astral}`),
      );
    });

    it('should order a tag before any longer tag it is a prefix of', () => {
      expect(hashORMapLeaf('key1', ['ab', 'a', ''], [])).toBe(hashString('key:key1||a|ab#'));
    });

    it('should return an unsigned 32-bit integer', () => {
      const hash = hashORMapLeaf('key1', ['tag-1'], ['dead-1']);

      expect(Number.isInteger(hash)).toBe(true);
      expect(hash).toBeGreaterThanOrEqual(0);
      expect(hash).toBeLessThanOrEqual(0xffffffff);
    });

    it('should not let a value contribute to the stored leaf', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const map1 = new ORMap<string, string>(new HLC('node-1'));
      map1.apply('key1', { value: 'value-a', timestamp: ts, tag: 'tag-1' });

      const map2 = new ORMap<string, string>(new HLC('node-1'));
      map2.apply('key1', { value: 'value-b', timestamp: ts, tag: 'tag-1' });

      expect(storedLeaf(map1, 'key1')).toBe(storedLeaf(map2, 'key1'));
      expect(storedLeaf(map1, 'key1')).toBe(hashORMapLeaf('key1', ['tag-1'], []));
    });

    it('should not let a timestamp contribute to the stored leaf', () => {
      const map1 = new ORMap<string, string>(new HLC('node-1'));
      map1.apply('key1', {
        value: 'value',
        timestamp: { millis: 1000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1',
      });

      const map2 = new ORMap<string, string>(new HLC('node-1'));
      map2.apply('key1', {
        value: 'value',
        timestamp: { millis: 2000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1',
      });

      expect(storedLeaf(map1, 'key1')).toBe(storedLeaf(map2, 'key1'));
    });

    it('should not let a TTL contribute to the stored leaf', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const withTtl = new ORMap<string, string>(new HLC('node-1'));
      withTtl.apply('key1', { value: 'value', timestamp: ts, tag: 'tag-1', ttlMs: 5000 });

      const noTtl = new ORMap<string, string>(new HLC('node-1'));
      noTtl.apply('key1', { value: 'value', timestamp: ts, tag: 'tag-1' });

      expect(storedLeaf(withTtl, 'key1')).toBe(storedLeaf(noTtl, 'key1'));
    });

    it('should keep an expired record in the stored leaf', () => {
      // The server holds a record until it is removed, whatever its TTL, so a
      // client that dropped expired tags from the leaf could never match it.
      const expired = new ORMap<string, string>(new HLC('node-1'));
      expired.apply('key1', {
        value: 'value',
        timestamp: { millis: 1, counter: 0, nodeId: 'node-1' },
        tag: 'tag-1',
        ttlMs: 1,
      });

      expect(expired.get('key1')).toEqual([]);
      expect(storedLeaf(expired, 'key1')).toBe(hashORMapLeaf('key1', ['tag-1'], []));
    });

    it('should handle object values deterministically, at every depth', () => {
      const ts = { millis: 1000, counter: 1, nodeId: 'node-1' };

      const map1 = new ORMap<string, unknown>(new HLC('node-1'));
      map1.apply('key1', { value: { a: 1, b: { z: 2 } }, timestamp: ts, tag: 'tag-1' });

      const map2 = new ORMap<string, unknown>(new HLC('node-1'));
      map2.apply('key1', { value: { b: { z: 999 }, a: 1 }, timestamp: ts, tag: 'tag-1' });

      expect(storedLeaf(map1, 'key1')).toBe(storedLeaf(map2, 'key1'));
    });

    it('should handle null values', () => {
      const map1 = new ORMap<string, null>(new HLC('node-1'));
      map1.apply('key1', {
        value: null,
        timestamp: { millis: 1000, counter: 1, nodeId: 'node-1' },
        tag: 'tag-1',
      });

      expect(storedLeaf(map1, 'key1')).toBe(hashORMapLeaf('key1', ['tag-1'], []));
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
      const nonEmptyBuckets = Object.values(buckets).filter((h) => h !== 0);
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

  describe('update', () => {
    it('should store the canonical leaf for live tags and tombstones', () => {
      tree.update('key1', ['tag-b', 'tag-a'], ['dead-a']);

      expect(tree.getEntryHashes(leafPathOf('key1')).get('key1')).toBe(
        hashORMapLeaf('key1', ['tag-a', 'tag-b'], ['dead-a']),
      );
      expect(tree.getRootHash()).not.toBe(0);
    });

    it('should keep a key that has only tombstones', () => {
      tree.update('key1', [], ['dead-a']);

      expect(tree.getEntryHashes(leafPathOf('key1')).get('key1')).toBe(
        hashString('key:key1|#dead-a'),
      );
    });

    it('should remove a key when both tag sets are empty', () => {
      tree.update('key1', ['tag-a'], ['dead-a']);
      tree.update('key1', [], []);

      expect(tree.getEntryHashes(leafPathOf('key1')).has('key1')).toBe(false);
      expect(tree.getRootHash()).toBe(0);
    });

    it('should never store a leaf for a key that was empty from the start', () => {
      tree.update('key1', [], []);

      expect(tree.getEntryHashes(leafPathOf('key1')).has('key1')).toBe(false);
      expect(tree.getRootHash()).toBe(0);
    });

    it('should change the root when only the tombstones change', () => {
      tree.update('key1', ['tag-a'], []);
      const before = tree.getRootHash();

      tree.update('key1', ['tag-a'], ['dead-a']);

      expect(tree.getRootHash()).not.toBe(before);
    });

    it('should accept sets and iterators', () => {
      const other = new ORMapMerkleTree();
      tree.update('key1', new Set(['tag-a']), new Set(['dead-a']));
      other.update('key1', new Map([['tag-a', 1]]).keys(), ['dead-a']);

      expect(tree.getRootHash()).toBe(other.getRootHash());
    });
  });

  describe('updateFromORMap parity with incremental updates', () => {
    it('should rebuild to the same root, tombstone-only keys included', () => {
      map.add('key1', 'value1');
      map.add('key1', 'value2');
      map.add('key2', 'value3');
      map.add('key3', 'value4');
      map.remove('key1', 'value1');
      // key3 is left with a tombstone and no record.
      map.remove('key3', 'value4');

      tree.updateFromORMap(map);

      expect(map.get('key3')).toEqual([]);
      expect(tree.getEntryHashes(leafPathOf('key3')).get('key3')).toBe(storedLeaf(map, 'key3'));
      expect(storedLeaf(map, 'key3')).toBeDefined();
      expect(tree.getRootHash()).toBe(map.getMerkleTree().getRootHash());
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
        tag: 'remote-tag-1',
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
        tag: localTag,
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
        tag: localTag,
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
        tag: 'different-tag',
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

      // Merge with tombstone for local tag — result not asserted, side effects verified below
      mapA.mergeKey('key1', [], [localTag]);

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
        tag: tombstonedTag,
      };

      // Merge with both record and its tombstone
      const result = mapA.mergeKey('key1', [remoteRecord], [tombstonedTag]);

      expect(result.added).toBe(0);
      expect(mapA.get('key1')).toHaveLength(0);
    });
  });
});

describe('ORMap tombstone-only key', () => {
  it('stays in the Merkle tree after its only value is removed, with a leaf that covers the tombstone', () => {
    const map = new ORMap<string, string>(new HLC('node-1'));
    const record = map.add('key1', 'only-value');
    map.remove('key1', 'only-value');

    const leafPath = hashString('key1').toString(16).padStart(8, '0').slice(0, 3);
    const leaf = map.getMerkleTree().getEntryHashes(leafPath).get('key1');

    // The server keeps a leaf for a key that holds only tombstones, so a client
    // that drops the key can never agree with it on the root. The expected value
    // is the TG-MRK-001 leaf with no live tags and the removed tag as the key's
    // only tombstone, written out here rather than taken from the code under test.
    expect(leaf).toBe(hashString(`key:key1|#${record.tag}`));
  });
});
