/**
 * Property-Based Tests for LWWMap
 *
 * These tests verify the mathematical properties that LWWMap must satisfy
 * as a Conflict-free Replicated Data Type (CRDT).
 *
 * Key CRDT Properties:
 * 1. Commutativity: merge(A, B) ≡ merge(B, A)
 * 2. Associativity: merge(A, merge(B, C)) ≡ merge(merge(A, B), C)
 * 3. Idempotence: merge(A, A) ≡ A
 * 4. Convergence: All replicas eventually converge to the same state
 */

import * as fc from 'fast-check';
import { LWWMap, LWWRecord } from '../LWWMap';
import { HLC, Timestamp } from '../HLC';

// Arbitrary generators for test data

const arbNodeId: fc.Arbitrary<string> = fc.string({ minLength: 1, maxLength: 10 })
  .filter(s => /^[a-z0-9]+$/.test(s));

const arbTimestamp: fc.Arbitrary<Timestamp> = fc.record({
  millis: fc.integer({ min: 1000000000000, max: 2000000000000 }), // Reasonable timestamp range
  counter: fc.integer({ min: 0, max: 1000 }),
  nodeId: arbNodeId,
});

const arbValue: fc.Arbitrary<any> = fc.oneof(
  fc.string(),
  fc.integer(),
  fc.boolean(),
  fc.constant(null),
  fc.record({
    name: fc.string(),
    count: fc.integer(),
  })
);

const arbRecord = (arbVal: fc.Arbitrary<any>): fc.Arbitrary<LWWRecord<any>> =>
  fc.record({
    value: arbVal,
    timestamp: arbTimestamp,
    ttlMs: fc.option(fc.integer({ min: 1000, max: 3600000 }), { nil: undefined }),
  });

const arbKey: fc.Arbitrary<string> = fc.string({ minLength: 1, maxLength: 5 })
  .filter(s => /^[a-z]+$/.test(s));

// Helper to create a fresh LWWMap
function createMap(nodeId: string = 'test-node'): LWWMap<string, any> {
  return new LWWMap(new HLC(nodeId));
}

// Helper to clone a map state
function cloneMapState(map: LWWMap<string, any>): Map<string, LWWRecord<any>> {
  const state = new Map<string, LWWRecord<any>>();
  for (const key of map.allKeys()) {
    const record = map.getRecord(key);
    if (record) {
      state.set(key, { ...record, timestamp: { ...record.timestamp } });
    }
  }
  return state;
}

// Helper to compare two maps for equality
function mapsEqual(a: LWWMap<string, any>, b: LWWMap<string, any>): boolean {
  const keysA = new Set(a.allKeys());
  const keysB = new Set(b.allKeys());

  if (keysA.size !== keysB.size) return false;

  for (const key of keysA) {
    if (!keysB.has(key)) return false;

    const recA = a.getRecord(key);
    const recB = b.getRecord(key);

    if (!recA || !recB) return false;
    if (recA.value !== recB.value) return false;
    if (HLC.compare(recA.timestamp, recB.timestamp) !== 0) return false;
  }

  return true;
}

describe('LWWMap Property-Based Tests', () => {
  describe('Idempotence', () => {
    it('merge(A, A) should not change state', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), (key, record) => {
          const map = createMap('node-1');

          // First merge
          map.merge(key, record);
          const stateAfterFirst = cloneMapState(map);

          // Second merge (same record)
          const changed = map.merge(key, record);
          const stateAfterSecond = cloneMapState(map);

          // State should be identical
          expect(changed).toBe(false);
          expect(stateAfterFirst.size).toBe(stateAfterSecond.size);

          for (const [k, rec] of stateAfterFirst) {
            const rec2 = stateAfterSecond.get(k);
            expect(rec2).toBeDefined();
            expect(rec.value).toEqual(rec2!.value);
            expect(HLC.compare(rec.timestamp, rec2!.timestamp)).toBe(0);
          }
        }),
        { numRuns: 100 }
      );
    });

    it('multiple identical merges should be idempotent', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), fc.integer({ min: 2, max: 10 }), (key, record, times) => {
          const map = createMap('node-1');

          // First merge
          map.merge(key, record);
          const initialState = cloneMapState(map);

          // Merge the same record multiple times
          for (let i = 0; i < times; i++) {
            map.merge(key, record);
          }

          const finalState = cloneMapState(map);

          // State should be identical to initial
          expect(initialState.size).toBe(finalState.size);
          for (const [k, rec] of initialState) {
            const rec2 = finalState.get(k);
            expect(rec2).toBeDefined();
            expect(rec.value).toEqual(rec2!.value);
          }
        }),
        { numRuns: 50 }
      );
    });
  });

  describe('Commutativity', () => {
    it('merge order should not affect final state for same key', () => {
      fc.assert(
        fc.property(arbKey, arbRecord(arbValue), arbRecord(arbValue), (key, recordA, recordB) => {
          // Map 1: merge A then B
          const map1 = createMap('node-1');
          map1.merge(key, recordA);
          map1.merge(key, recordB);

          // Map 2: merge B then A
          const map2 = createMap('node-2');
          map2.merge(key, recordB);
          map2.merge(key, recordA);

          // Both should have the same value for the key
          const rec1 = map1.getRecord(key);
          const rec2 = map2.getRecord(key);

          expect(rec1).toBeDefined();
          expect(rec2).toBeDefined();
          expect(rec1!.value).toEqual(rec2!.value);
          expect(HLC.compare(rec1!.timestamp, rec2!.timestamp)).toBe(0);
        }),
        { numRuns: 100 }
      );
    });

    it('merge order should not affect final state for multiple keys', () => {
      fc.assert(
        fc.property(
          fc.array(fc.tuple(arbKey, arbRecord(arbValue)), { minLength: 1, maxLength: 10 }),
          (operations) => {
            // Map 1: original order
            const map1 = createMap('node-1');
            for (const [key, record] of operations) {
              map1.merge(key, record);
            }

            // Map 2: reversed order
            const map2 = createMap('node-2');
            for (const [key, record] of [...operations].reverse()) {
              map2.merge(key, record);
            }

            // Both should converge to same state
            expect(mapsEqual(map1, map2)).toBe(true);
          }
        ),
        { numRuns: 50 }
      );
    });
  });

  describe('Associativity', () => {
    it('merge(A, merge(B, C)) === merge(merge(A, B), C) for same key', () => {
      fc.assert(
        fc.property(
          arbKey,
          arbRecord(arbValue),
          arbRecord(arbValue),
          arbRecord(arbValue),
          (key, recordA, recordB, recordC) => {
            // Left associative: ((A merge B) merge C)
            const mapLeft = createMap('node-left');
            mapLeft.merge(key, recordA);
            mapLeft.merge(key, recordB);
            mapLeft.merge(key, recordC);

            // Right associative: (A merge (B merge C))
            // First prepare B merge C
            const tempBC = createMap('node-temp');
            tempBC.merge(key, recordB);
            tempBC.merge(key, recordC);
            const recordBC = tempBC.getRecord(key);

            const mapRight = createMap('node-right');
            mapRight.merge(key, recordA);
            if (recordBC) {
              mapRight.merge(key, recordBC);
            }

            // Results should be equal
            const recLeft = mapLeft.getRecord(key);
            const recRight = mapRight.getRecord(key);

            expect(recLeft).toBeDefined();
            expect(recRight).toBeDefined();
            expect(recLeft!.value).toEqual(recRight!.value);
            expect(HLC.compare(recLeft!.timestamp, recRight!.timestamp)).toBe(0);
          }
        ),
        { numRuns: 100 }
      );
    });
  });

  describe('Convergence', () => {
    it('all replicas should converge after exchanging all operations', () => {
      fc.assert(
        fc.property(
          fc.array(fc.tuple(arbKey, arbRecord(arbValue)), { minLength: 1, maxLength: 20 }),
          fc.integer({ min: 2, max: 5 }),
          (operations, numReplicas) => {
            // Create replicas
            const replicas = Array.from({ length: numReplicas }, (_, i) =>
              createMap(`replica-${i}`)
            );

            // Each replica receives operations in a different shuffled order
            for (let i = 0; i < numReplicas; i++) {
              // Shuffle operations differently for each replica
              const shuffled = [...operations].sort(() => Math.random() - 0.5);
              for (const [key, record] of shuffled) {
                replicas[i].merge(key, record);
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

    it('replicas should converge even with concurrent writes', () => {
      fc.assert(
        fc.property(
          arbKey,
          fc.array(arbValue, { minLength: 2, maxLength: 5 }),
          (key, values) => {
            // Each "node" writes a different value with its own timestamp
            const records: LWWRecord<any>[] = values.map((value, i) => ({
              value,
              timestamp: {
                millis: Date.now() + i, // Slightly different times
                counter: 0,
                nodeId: `node-${i}`,
              },
            }));

            // Create two replicas
            const replica1 = createMap('replica-1');
            const replica2 = createMap('replica-2');

            // Replica 1 receives in order
            for (const record of records) {
              replica1.merge(key, record);
            }

            // Replica 2 receives in reverse order
            for (const record of [...records].reverse()) {
              replica2.merge(key, record);
            }

            // Should converge to the same value
            const rec1 = replica1.getRecord(key);
            const rec2 = replica2.getRecord(key);

            expect(rec1).toBeDefined();
            expect(rec2).toBeDefined();
            expect(rec1!.value).toEqual(rec2!.value);
            expect(HLC.compare(rec1!.timestamp, rec2!.timestamp)).toBe(0);
          }
        ),
        { numRuns: 50 }
      );
    });
  });

  describe('LWW Semantics', () => {
    it('later timestamp should always win', () => {
      fc.assert(
        fc.property(arbKey, arbValue, arbValue, (key, value1, value2) => {
          const map = createMap('node-1');

          const earlier: LWWRecord<any> = {
            value: value1,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-a' },
          };

          const later: LWWRecord<any> = {
            value: value2,
            timestamp: { millis: 2000, counter: 0, nodeId: 'node-a' },
          };

          // Merge later first
          map.merge(key, later);
          // Then merge earlier
          const changed = map.merge(key, earlier);

          // Earlier should not win
          expect(changed).toBe(false);
          expect(map.getRecord(key)!.value).toEqual(value2);
        }),
        { numRuns: 100 }
      );
    });

    it('equal millis: higher counter should win', () => {
      fc.assert(
        fc.property(arbKey, arbValue, arbValue, (key, value1, value2) => {
          const map = createMap('node-1');

          const lowerCounter: LWWRecord<any> = {
            value: value1,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-a' },
          };

          const higherCounter: LWWRecord<any> = {
            value: value2,
            timestamp: { millis: 1000, counter: 5, nodeId: 'node-a' },
          };

          // Merge higher counter first
          map.merge(key, higherCounter);
          // Then merge lower counter
          const changed = map.merge(key, lowerCounter);

          // Lower counter should not win
          expect(changed).toBe(false);
          expect(map.getRecord(key)!.value).toEqual(value2);
        }),
        { numRuns: 100 }
      );
    });

    it('equal millis and counter: nodeId should be tiebreaker', () => {
      fc.assert(
        fc.property(arbKey, arbValue, arbValue, (key, value1, value2) => {
          const map = createMap('node-1');

          const nodeA: LWWRecord<any> = {
            value: value1,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-a' },
          };

          const nodeZ: LWWRecord<any> = {
            value: value2,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-z' },
          };

          // Both have same millis and counter, nodeId is tiebreaker
          // 'node-z' > 'node-a' lexicographically

          map.merge(key, nodeA);
          const changed = map.merge(key, nodeZ);

          // node-z should win (greater nodeId)
          expect(changed).toBe(true);
          expect(map.getRecord(key)!.value).toEqual(value2);
        }),
        { numRuns: 50 }
      );
    });
  });

  describe('Tombstone Handling', () => {
    it('tombstone should be treated as a valid value (null)', () => {
      fc.assert(
        fc.property(arbKey, arbValue, (key, value) => {
          const map = createMap('node-1');

          const liveRecord: LWWRecord<any> = {
            value,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-a' },
          };

          const tombstone: LWWRecord<any> = {
            value: null,
            timestamp: { millis: 2000, counter: 0, nodeId: 'node-a' },
          };

          map.merge(key, liveRecord);
          map.merge(key, tombstone);

          // get() should return undefined for tombstone
          expect(map.get(key)).toBeUndefined();
          // But getRecord() should return the tombstone
          expect(map.getRecord(key)).toBeDefined();
          expect(map.getRecord(key)!.value).toBeNull();
        }),
        { numRuns: 50 }
      );
    });

    it('later live value should resurrect tombstoned key', () => {
      fc.assert(
        fc.property(arbKey, arbValue, (key, value) => {
          fc.pre(value !== null); // Ensure value is not null

          const map = createMap('node-1');

          const tombstone: LWWRecord<any> = {
            value: null,
            timestamp: { millis: 1000, counter: 0, nodeId: 'node-a' },
          };

          const resurrection: LWWRecord<any> = {
            value,
            timestamp: { millis: 2000, counter: 0, nodeId: 'node-a' },
          };

          map.merge(key, tombstone);
          expect(map.get(key)).toBeUndefined();

          map.merge(key, resurrection);
          expect(map.get(key)).toEqual(value);
        }),
        { numRuns: 50 }
      );
    });
  });
});
