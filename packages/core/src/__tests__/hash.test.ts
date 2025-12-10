import { hashString, combineHashes } from '../utils/hash';

describe('Hash utilities', () => {
  describe('hashString', () => {
    describe('Basic operations', () => {
      test('should hash a simple string', () => {
        const hash = hashString('hello');
        expect(typeof hash).toBe('number');
        expect(hash).toBeGreaterThanOrEqual(0);
      });

      test('should return positive 32-bit integer', () => {
        const hash = hashString('test');
        expect(hash).toBeGreaterThanOrEqual(0);
        expect(hash).toBeLessThanOrEqual(0xFFFFFFFF);
      });

      test('should hash different strings to different hashes', () => {
        const hash1 = hashString('hello');
        const hash2 = hashString('world');
        expect(hash1).not.toBe(hash2);
      });

      test('should hash long strings', () => {
        const longString = 'a'.repeat(10000);
        const hash = hashString(longString);
        expect(typeof hash).toBe('number');
        expect(hash).toBeGreaterThanOrEqual(0);
      });
    });

    describe('Consistency', () => {
      test('same string should produce same hash (deterministic)', () => {
        const str = 'consistent-string';
        const hash1 = hashString(str);
        const hash2 = hashString(str);
        const hash3 = hashString(str);
        expect(hash1).toBe(hash2);
        expect(hash2).toBe(hash3);
      });

      test('slightly different strings should produce different hashes', () => {
        const hash1 = hashString('test1');
        const hash2 = hashString('test2');
        expect(hash1).not.toBe(hash2);
      });

      test('same content with different casing should produce different hashes', () => {
        const hash1 = hashString('Hello');
        const hash2 = hashString('hello');
        const hash3 = hashString('HELLO');
        expect(hash1).not.toBe(hash2);
        expect(hash2).not.toBe(hash3);
        expect(hash1).not.toBe(hash3);
      });

      test('strings with whitespace differences should produce different hashes', () => {
        const hash1 = hashString('hello world');
        const hash2 = hashString('hello  world');
        const hash3 = hashString(' hello world');
        expect(hash1).not.toBe(hash2);
        expect(hash1).not.toBe(hash3);
      });
    });

    describe('Edge cases', () => {
      test('should hash empty string', () => {
        const hash = hashString('');
        expect(typeof hash).toBe('number');
        expect(hash).toBeGreaterThanOrEqual(0);
      });

      test('empty string should produce consistent hash', () => {
        const hash1 = hashString('');
        const hash2 = hashString('');
        expect(hash1).toBe(hash2);
      });

      test('should hash single character', () => {
        const hash = hashString('a');
        expect(typeof hash).toBe('number');
        expect(hash).toBeGreaterThanOrEqual(0);
      });

      test('should handle unicode characters', () => {
        const hash1 = hashString('привет');
        const hash2 = hashString('こんにちは');
        const hash3 = hashString('مرحبا');
        const hash4 = hashString('🚀🎉🔥');

        expect(typeof hash1).toBe('number');
        expect(typeof hash2).toBe('number');
        expect(typeof hash3).toBe('number');
        expect(typeof hash4).toBe('number');

        // All different
        expect(hash1).not.toBe(hash2);
        expect(hash2).not.toBe(hash3);
        expect(hash3).not.toBe(hash4);
      });

      test('should handle special characters', () => {
        const hash1 = hashString('hello\nworld');
        const hash2 = hashString('hello\tworld');
        const hash3 = hashString('hello\0world');

        expect(hash1).not.toBe(hash2);
        expect(hash2).not.toBe(hash3);
      });

      test('should handle strings with quotes and escapes', () => {
        const hash1 = hashString('"quoted"');
        const hash2 = hashString("'single'");
        const hash3 = hashString('back\\slash');

        expect(typeof hash1).toBe('number');
        expect(typeof hash2).toBe('number');
        expect(typeof hash3).toBe('number');
      });

      test('should handle JSON strings', () => {
        const json1 = JSON.stringify({ name: 'Alice', age: 30 });
        const json2 = JSON.stringify({ name: 'Bob', age: 25 });

        const hash1 = hashString(json1);
        const hash2 = hashString(json2);

        expect(hash1).not.toBe(hash2);
      });

      test('should handle JSON objects with same keys different order', () => {
        // JSON.stringify preserves insertion order, so these will be different strings
        const json1 = JSON.stringify({ a: 1, b: 2 });
        const json2 = JSON.stringify({ b: 2, a: 1 });

        // If strings are different, hashes will be different
        if (json1 !== json2) {
          expect(hashString(json1)).not.toBe(hashString(json2));
        } else {
          expect(hashString(json1)).toBe(hashString(json2));
        }
      });
    });

    describe('Distribution (hash quality)', () => {
      test('should produce varied hashes for sequential numbers', () => {
        const hashes = new Set<number>();
        for (let i = 0; i < 1000; i++) {
          hashes.add(hashString(`item-${i}`));
        }
        // All 1000 hashes should be unique
        expect(hashes.size).toBe(1000);
      });

      test('should produce varied hashes for similar strings', () => {
        const hashes = new Set<number>();
        const prefixes = ['a', 'b', 'c', 'd', 'e'];

        for (const prefix of prefixes) {
          for (let i = 0; i < 100; i++) {
            hashes.add(hashString(`${prefix}${i}`));
          }
        }
        // All 500 hashes should be unique
        expect(hashes.size).toBe(500);
      });
    });
  });

  describe('combineHashes', () => {
    describe('Basic operations', () => {
      test('should combine two hashes', () => {
        const h1 = hashString('hello');
        const h2 = hashString('world');
        const combined = combineHashes([h1, h2]);

        expect(typeof combined).toBe('number');
        expect(combined).toBeGreaterThanOrEqual(0);
      });

      test('should combine multiple hashes', () => {
        const hashes = [
          hashString('a'),
          hashString('b'),
          hashString('c'),
          hashString('d'),
          hashString('e')
        ];
        const combined = combineHashes(hashes);

        expect(typeof combined).toBe('number');
        expect(combined).toBeGreaterThanOrEqual(0);
      });

      test('should return positive 32-bit integer', () => {
        const hashes = [0xFFFFFFFF, 0xFFFFFFFF];
        const combined = combineHashes(hashes);

        expect(combined).toBeGreaterThanOrEqual(0);
        expect(combined).toBeLessThanOrEqual(0xFFFFFFFF);
      });
    });

    describe('Order independence', () => {
      test('should produce same result regardless of order (commutative)', () => {
        const h1 = hashString('first');
        const h2 = hashString('second');
        const h3 = hashString('third');

        const combined1 = combineHashes([h1, h2, h3]);
        const combined2 = combineHashes([h3, h1, h2]);
        const combined3 = combineHashes([h2, h3, h1]);

        expect(combined1).toBe(combined2);
        expect(combined2).toBe(combined3);
      });
    });

    describe('Edge cases', () => {
      test('should handle empty array', () => {
        const combined = combineHashes([]);
        expect(combined).toBe(0);
      });

      test('should handle single hash', () => {
        const hash = hashString('single');
        const combined = combineHashes([hash]);
        expect(combined).toBe(hash >>> 0);
      });

      test('should handle array with zero', () => {
        const h1 = hashString('test');
        const combined1 = combineHashes([h1, 0]);
        const combined2 = combineHashes([0, h1]);

        expect(combined1).toBe(combined2);
        expect(combined1).toBe(h1 >>> 0);
      });

      test('should handle large numbers', () => {
        const largeHashes = [0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF];
        const combined = combineHashes(largeHashes);

        expect(typeof combined).toBe('number');
        expect(combined).toBeGreaterThanOrEqual(0);
        expect(combined).toBeLessThanOrEqual(0xFFFFFFFF);
      });

      test('should handle many hashes', () => {
        const hashes: number[] = [];
        for (let i = 0; i < 10000; i++) {
          hashes.push(hashString(`item-${i}`));
        }
        const combined = combineHashes(hashes);

        expect(typeof combined).toBe('number');
        expect(combined).toBeGreaterThanOrEqual(0);
      });
    });

    describe('Consistency', () => {
      test('same hashes should produce same combined result', () => {
        const hashes = [hashString('a'), hashString('b'), hashString('c')];

        const combined1 = combineHashes(hashes);
        const combined2 = combineHashes(hashes);
        const combined3 = combineHashes([...hashes]);

        expect(combined1).toBe(combined2);
        expect(combined2).toBe(combined3);
      });

      test('different sets of hashes should produce different results', () => {
        const set1 = [hashString('a'), hashString('b')];
        const set2 = [hashString('c'), hashString('d')];
        const set3 = [hashString('a'), hashString('c')];

        const combined1 = combineHashes(set1);
        const combined2 = combineHashes(set2);
        const combined3 = combineHashes(set3);

        expect(combined1).not.toBe(combined2);
        expect(combined1).not.toBe(combined3);
        expect(combined2).not.toBe(combined3);
      });
    });

    describe('Use cases', () => {
      test('combining bucket hashes for Merkle tree', () => {
        // Simulate bucket hashes from different data shards
        const bucketHashes = [
          hashString(JSON.stringify({ id: 1, data: 'shard1' })),
          hashString(JSON.stringify({ id: 2, data: 'shard2' })),
          hashString(JSON.stringify({ id: 3, data: 'shard3' })),
          hashString(JSON.stringify({ id: 4, data: 'shard4' }))
        ];

        const rootHash = combineHashes(bucketHashes);
        expect(typeof rootHash).toBe('number');

        // If any bucket changes, root should change
        const modifiedBucketHashes = [
          hashString(JSON.stringify({ id: 1, data: 'shard1' })),
          hashString(JSON.stringify({ id: 2, data: 'MODIFIED' })), // Changed
          hashString(JSON.stringify({ id: 3, data: 'shard3' })),
          hashString(JSON.stringify({ id: 4, data: 'shard4' }))
        ];

        const newRootHash = combineHashes(modifiedBucketHashes);
        expect(newRootHash).not.toBe(rootHash);
      });

      test('detecting data synchronization differences', () => {
        // Client and server have their data hashes
        const clientRecords = ['record1', 'record2', 'record3'];
        const serverRecords = ['record1', 'record2', 'record3'];

        const clientHash = combineHashes(clientRecords.map(hashString));
        const serverHash = combineHashes(serverRecords.map(hashString));

        // Same records = same hash
        expect(clientHash).toBe(serverHash);

        // Add record to server
        const serverRecordsUpdated = [...serverRecords, 'record4'];
        const serverHashUpdated = combineHashes(serverRecordsUpdated.map(hashString));

        // Different records = different hash
        expect(clientHash).not.toBe(serverHashUpdated);
      });
    });
  });
});
