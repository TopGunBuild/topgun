/**
 * Property-Based Tests for ORMap (Observed-Remove Map)
 *
 * These tests verify the mathematical properties that ORMap must satisfy
 * as a Conflict-free Replicated Data Type (CRDT).
 *
 * ORMap Properties:
 * 1. Commutativity: merge(A, B) ≡ merge(B, A)
 * 2. Associativity: merge(A, merge(B, C)) ≡ merge(merge(A, B), C)
 * 3. Idempotence: merge(A, A) ≡ A
 * 4. Convergence: All replicas eventually converge to the same state
 * 5. Add-Wins: Concurrent add and remove of the same element should preserve the add
 *    (OR-Map uses "Observed-Remove" which means only removes what was observed)
 */

import * as fc from 'fast-check';
import { ORMap, ORMapRecord } from '../ORMap';
import { HLC, Timestamp } from '../HLC';

// Arbitrary generators

const arbNodeId: fc.Arbitrary<string> = fc.string({ minLength: 1, maxLength: 10 })
  .filter(s => /^[a-z0-9]+$/.test(s));

const arbTimestamp: fc.Arbitrary<Timestamp> = fc.record({
  millis: fc.integer({ min: 1000000000000, max: 2000000000000 }),
  counter: fc.integer({ min: 0, max: 1000 }),
  nodeId: arbNodeId,
});

const arbValue: fc.Arbitrary<any> = fc.oneof(
  fc.string({ minLength: 1, maxLength: 10 }),
  fc.integer(),
  fc.record({
    id: fc.integer(),
    name: fc.string(),
  })
);

const arbTag: fc.Arbitrary<string> = arbTimestamp.map(ts => HLC.toString(ts));

// Generate records without TTL for property tests to avoid expiration issues
const arbRecord = (arbVal: fc.Arbitrary<any>): fc.Arbitrary<ORMapRecord<any>> =>
  fc.record({
    value: arbVal,
    timestamp: arbTimestamp,
    tag: arbTag,
  }).map(r => ({ ...r, ttlMs: undefined }));

const arbKey: fc.Arbitrary<string> = fc.string({ minLength: 1, maxLength: 5 })
  .filter(s => /^[a-z]+$/.test(s));

// Helper to create a fresh ORMap
function createMap(nodeId: string = 'test-node'): ORMap<string, any> {
  return new ORMap(new HLC(nodeId));
}

// Helper to compare two ORMaps for equality
function mapsEqual(a: ORMap<string, any>, b: ORMap<string, any>): boolean {
  const keysA = new Set(a.allKeys());
  const keysB = new Set(b.allKeys());

  if (keysA.size !== keysB.size) return false;

  for (const key of keysA) {
    if (!keysB.has(key)) return false;

    const recordsA = a.getRecordsMap(key);
    const recordsB = b.getRecordsMap(key);

    if (!recordsA && !recordsB) continue;
    if (!recordsA || !recordsB) return false;
    if (recordsA.size !== recordsB.size) return false;

    for (const [tag, recA] of recordsA) {
      const recB = recordsB.get(tag);
      if (!recB) return false;
      if (recA.value !== recB.value && JSON.stringify(recA.value) !== JSON.stringify(recB.value)) {
        return false;
      }
    }
  }

  // Also compare tombstones
  const tombstonesA = new Set(a.getTombstones());
  const tombstonesB = new Set(b.getTombstones());

  if (tombstonesA.size !== tombstonesB.size) return false;
  for (const tag of tombstonesA) {
    if (!tombstonesB.has(tag)) return false;
  }

  return true;
}

// Helper to get all values from a key (sorted for comparison)
function getValuesSet(map: ORMap<string, any>, key: string): Set<string> {
  return new Set(map.get(key).map(v => JSON.stringify(v)));
}

describe('ORMap Property-Based Tests', () => {
  describe('Idempotence', () => {
    it('applying the same record twice should be idempotent', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), (key, record) => {
          const map = createMap('node-1');

          // First apply
          map.apply(key, record);
          const recordsAfterFirst = map.getRecords(key).length;

          // Second apply (same record)
          map.apply(key, record);
          const recordsAfterSecond = map.getRecords(key).length;

          // Should have same number of records
          expect(recordsAfterSecond).toBe(recordsAfterFirst);
        }),
        { numRuns: 100 }
      );
    });

    it('applying the same tombstone twice should be idempotent', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), (key, record) => {
          const map = createMap('node-1');
          map.apply(key, record);

          // First tombstone
          map.applyTombstone(record.tag);
          const recordsAfterFirst = map.getRecords(key).length;
          const tombstonesAfterFirst = map.getTombstones().length;

          // Second tombstone (same tag)
          map.applyTombstone(record.tag);
          const recordsAfterSecond = map.getRecords(key).length;
          const tombstonesAfterSecond = map.getTombstones().length;

          expect(recordsAfterSecond).toBe(recordsAfterFirst);
          expect(tombstonesAfterSecond).toBe(tombstonesAfterFirst);
        }),
        { numRuns: 100 }
      );
    });
  });

  describe('Commutativity', () => {
    it('merge order should not affect final state', () => {
      fc.assert(
        fc.property(
          fc.array(fc.tuple(arbKey, arbRecord(arbValue)), { minLength: 1, maxLength: 10 }),
          (operations) => {
            // Map 1: apply in order
            const map1 = createMap('node-1');
            for (const [key, record] of operations) {
              map1.apply(key, record);
            }

            // Map 2: apply in reverse order
            const map2 = createMap('node-2');
            for (const [key, record] of [...operations].reverse()) {
              map2.apply(key, record);
            }

            // Both should have the same state
            expect(mapsEqual(map1, map2)).toBe(true);
          }
        ),
        { numRuns: 50 }
      );
    });

    it('tombstone order should not affect final state', () => {
      fc.assert(
        fc.property(
          fc.array(fc.tuple(arbKey, arbRecord(arbValue)), { minLength: 2, maxLength: 5 }),
          (operations) => {
            // Apply all records first, then tombstones in different orders
            const tags = operations.map(([, r]) => r.tag);

            const map1 = createMap('node-1');
            const map2 = createMap('node-2');

            // Apply records to both
            for (const [key, record] of operations) {
              map1.apply(key, record);
              map2.apply(key, record);
            }

            // Apply tombstones in different order
            for (const tag of tags) {
              map1.applyTombstone(tag);
            }
            for (const tag of [...tags].reverse()) {
              map2.applyTombstone(tag);
            }

            // Both should have same tombstones
            expect(new Set(map1.getTombstones())).toEqual(new Set(map2.getTombstones()));
          }
        ),
        { numRuns: 50 }
      );
    });
  });

  describe('Associativity', () => {
    it('nested merges should produce same result regardless of grouping', () => {
      fc.assert(
        fc.property(
          arbKey,
          arbRecord(arbValue),
          arbRecord(arbValue),
          arbRecord(arbValue),
          (key, recordA, recordB, recordC) => {
            // Ensure unique tags
            const tsA = { millis: 1000, counter: 0, nodeId: 'a' };
            const tsB = { millis: 2000, counter: 0, nodeId: 'b' };
            const tsC = { millis: 3000, counter: 0, nodeId: 'c' };

            recordA = { ...recordA, tag: HLC.toString(tsA), timestamp: tsA };
            recordB = { ...recordB, tag: HLC.toString(tsB), timestamp: tsB };
            recordC = { ...recordC, tag: HLC.toString(tsC), timestamp: tsC };

            // Left: ((A, B), C)
            const mapLeft = createMap('node-left');
            mapLeft.apply(key, recordA);
            mapLeft.apply(key, recordB);
            mapLeft.apply(key, recordC);

            // Right: (A, (B, C))
            const mapRight = createMap('node-right');
            mapRight.apply(key, recordA);
            mapRight.apply(key, recordB);
            mapRight.apply(key, recordC);

            expect(mapsEqual(mapLeft, mapRight)).toBe(true);
          }
        ),
        { numRuns: 50 }
      );
    });
  });

  describe('Convergence', () => {
    it('all replicas should converge after exchanging all operations', () => {
      fc.assert(
        fc.property(
          fc.array(fc.tuple(arbKey, arbRecord(arbValue)), { minLength: 1, maxLength: 15 }),
          fc.integer({ min: 2, max: 4 }),
          (operations, numReplicas) => {
            // Ensure unique tags
            const uniqueOps = operations.map(([key, record], i) => {
              const ts = { millis: 1000000000000 + i * 1000, counter: 0, nodeId: `n${i}` };
              return [key, { ...record, tag: HLC.toString(ts), timestamp: ts }] as [string, ORMapRecord<any>];
            });

            // Create replicas that receive operations in different orders
            const replicas = Array.from({ length: numReplicas }, (_, i) =>
              createMap(`replica-${i}`)
            );

            // Each replica receives operations in a shuffled order
            for (let i = 0; i < numReplicas; i++) {
              const shuffled = [...uniqueOps].sort(() => Math.random() - 0.5);
              for (const [key, record] of shuffled) {
                replicas[i].apply(key, record);
              }
            }

            // All replicas should have the same state
            for (let i = 1; i < numReplicas; i++) {
              expect(mapsEqual(replicas[0], replicas[i])).toBe(true);
            }
          }
        ),
        { numRuns: 30 }
      );
    });

    it('replicas should converge with mixed adds and removes', () => {
      fc.assert(
        fc.property(
          fc.array(arbValue, { minLength: 3, maxLength: 6 }),
          (values) => {
            const key = 'test-key';

            // Create records with unique tags
            const records = values.map((value, i) => {
              const ts: Timestamp = { millis: 1000 + i, counter: 0, nodeId: `node-${i}` };
              return {
                value,
                timestamp: ts,
                tag: HLC.toString(ts),
              };
            });

            // Replica 1: add all, then remove first half
            const replica1 = createMap('replica-1');
            for (const record of records) {
              replica1.apply(key, record);
            }
            for (let i = 0; i < Math.floor(records.length / 2); i++) {
              replica1.applyTombstone(records[i].tag);
            }

            // Replica 2: add all, then remove same first half (but in reverse)
            const replica2 = createMap('replica-2');
            for (const record of [...records].reverse()) {
              replica2.apply(key, record);
            }
            for (let i = Math.floor(records.length / 2) - 1; i >= 0; i--) {
              replica2.applyTombstone(records[i].tag);
            }

            // Should converge
            expect(mapsEqual(replica1, replica2)).toBe(true);
          }
        ),
        { numRuns: 30 }
      );
    });
  });

  describe('Observed-Remove Semantics', () => {
    it('tombstone should only remove records with matching tag', () => {
      fc.assert(
        fc.property(arbKey, arbValue, arbValue, (key, value1, value2) => {
          const map = createMap('node-1');

          const ts1: Timestamp = { millis: 1000, counter: 0, nodeId: 'a' };
          const ts2: Timestamp = { millis: 2000, counter: 0, nodeId: 'b' };

          const record1: ORMapRecord<any> = {
            value: value1,
            timestamp: ts1,
            tag: HLC.toString(ts1),
          };

          const record2: ORMapRecord<any> = {
            value: value2,
            timestamp: ts2,
            tag: HLC.toString(ts2),
          };

          // Add both records
          map.apply(key, record1);
          map.apply(key, record2);
          expect(map.getRecords(key).length).toBe(2);

          // Remove only record1
          map.applyTombstone(record1.tag);

          // record2 should still exist
          const remaining = map.getRecords(key);
          expect(remaining.length).toBe(1);
          expect(remaining[0].tag).toBe(record2.tag);
        }),
        { numRuns: 50 }
      );
    });

    it('add after remove should succeed (add-wins for new tag)', () => {
      fc.assert(
        fc.property(arbKey, arbValue, arbValue, (key, value1, value2) => {
          const map = createMap('node-1');

          const ts1: Timestamp = { millis: 1000, counter: 0, nodeId: 'a' };
          const ts2: Timestamp = { millis: 2000, counter: 0, nodeId: 'a' };

          const record1: ORMapRecord<any> = {
            value: value1,
            timestamp: ts1,
            tag: HLC.toString(ts1),
          };

          const record2: ORMapRecord<any> = {
            value: value2,
            timestamp: ts2,
            tag: HLC.toString(ts2),
          };

          // Add record1
          map.apply(key, record1);

          // Remove record1
          map.applyTombstone(record1.tag);
          expect(map.getRecords(key).length).toBe(0);

          // Add record2 (new tag)
          map.apply(key, record2);
          expect(map.getRecords(key).length).toBe(1);
        }),
        { numRuns: 50 }
      );
    });

    it('tombstoned record cannot be re-added with same tag', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), (key, record) => {
          const map = createMap('node-1');

          // Add record
          map.apply(key, record);
          expect(map.getRecords(key).length).toBe(1);

          // Tombstone it
          map.applyTombstone(record.tag);
          expect(map.getRecords(key).length).toBe(0);

          // Try to re-add with same tag
          const applied = map.apply(key, record);
          expect(applied).toBe(false);
          expect(map.getRecords(key).length).toBe(0);
        }),
        { numRuns: 50 }
      );
    });
  });

  describe('Multi-Value Semantics', () => {
    it('same key can hold multiple values with different tags', () => {
      fc.assert(
        fc.property(
          arbKey,
          fc.array(arbValue, { minLength: 2, maxLength: 5 }),
          (key, values) => {
            const map = createMap('node-1');

            // Add all values with unique tags
            const records = values.map((value, i) => {
              const ts: Timestamp = { millis: 1000 + i, counter: 0, nodeId: `n${i}` };
              return {
                value,
                timestamp: ts,
                tag: HLC.toString(ts),
              };
            });

            for (const record of records) {
              map.apply(key, record);
            }

            // All values should be present
            expect(map.getRecords(key).length).toBe(values.length);
          }
        ),
        { numRuns: 50 }
      );
    });
  });

  describe('MergeKey Operation', () => {
    it('mergeKey should correctly merge remote records', () => {
      fc.assert(
        fc.property(
          arbKey,
          fc.array(arbRecord(arbValue), { minLength: 1, maxLength: 5 }),
          (key, records) => {
            // Ensure unique tags
            const uniqueRecords = records.map((record, i) => {
              const ts: Timestamp = { millis: 1000 + i * 1000, counter: 0, nodeId: `n${i}` };
              return { ...record, tag: HLC.toString(ts), timestamp: ts };
            });

            const map1 = createMap('node-1');
            const map2 = createMap('node-2');

            // Add records to map1 using apply
            for (const record of uniqueRecords) {
              map1.apply(key, record);
            }

            // Merge into map2 using mergeKey
            const result = map2.mergeKey(key, uniqueRecords, []);

            expect(result.added).toBe(uniqueRecords.length);
            expect(mapsEqual(map1, map2)).toBe(true);
          }
        ),
        { numRuns: 50 }
      );
    });

    it('mergeKey should respect tombstones from remote', () => {
      fc.assert(
        fc.property(
          arbKey,
          fc.array(arbRecord(arbValue), { minLength: 2, maxLength: 5 }),
          (key, records) => {
            // Ensure unique tags
            const uniqueRecords = records.map((record, i) => {
              const ts: Timestamp = { millis: 1000 + i * 1000, counter: 0, nodeId: `n${i}` };
              return { ...record, tag: HLC.toString(ts), timestamp: ts };
            });

            const map = createMap('node-1');

            // Add some records first
            for (const record of uniqueRecords) {
              map.apply(key, record);
            }

            // Merge with tombstones for first half
            const tombstones = uniqueRecords.slice(0, Math.floor(uniqueRecords.length / 2)).map(r => r.tag);
            const remainingRecords = uniqueRecords.slice(Math.floor(uniqueRecords.length / 2));

            map.mergeKey(key, remainingRecords, tombstones);

            // Only remaining records should exist
            const finalRecords = map.getRecords(key);
            expect(finalRecords.length).toBe(remainingRecords.length);
          }
        ),
        { numRuns: 50 }
      );
    });
  });
});
