/**
 * FilteringResultSet Tests
 */

import { FilteringResultSet } from '../../../query/resultset/FilteringResultSet';
import { SetResultSet } from '../../../query/resultset/SetResultSet';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  active: boolean;
}

describe('FilteringResultSet', () => {
  const records: Map<string, TestRecord> = new Map([
    ['1', { id: '1', name: 'Alice', age: 30, active: true }],
    ['2', { id: '2', name: 'Bob', age: 25, active: false }],
    ['3', { id: '3', name: 'Charlie', age: 35, active: true }],
    ['4', { id: '4', name: 'Diana', age: 28, active: true }],
    ['5', { id: '5', name: 'Eve', age: 40, active: false }],
  ]);

  const getRecord = (key: string) => records.get(key);

  describe('filtering logic', () => {
    it('should filter by predicate', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.toArray().sort()).toEqual(['1', '3', '4']);
    });

    it('should filter by age predicate', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.age >= 30
      );

      expect(result.toArray().sort()).toEqual(['1', '3', '5']);
    });

    it('should filter by complex predicate', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active && r.age < 35
      );

      expect(result.toArray().sort()).toEqual(['1', '4']);
    });

    it('should handle empty source', () => {
      const source = new SetResultSet(new Set<string>(), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should handle predicate matching nothing', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.age > 100
      );

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should handle predicate matching everything', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        () => true
      );

      expect(result.toArray().sort()).toEqual(['1', '2', '3']);
    });

    it('should skip records that do not exist', () => {
      const source = new SetResultSet(new Set(['1', '99', '2']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.toArray().sort()).toEqual(['1']);
    });
  });

  describe('contains', () => {
    it('should check source and predicate', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.contains('1')).toBe(true); // In source, passes predicate
      expect(result.contains('2')).toBe(false); // In source, fails predicate
      expect(result.contains('99')).toBe(false); // Not in source
    });

    it('should return false for missing records', () => {
      const source = new SetResultSet(new Set(['99']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        () => true
      );

      expect(result.contains('99')).toBe(false);
    });
  });

  describe('size', () => {
    it('should return correct filtered size', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.size()).toBe(3);
    });
  });

  describe('isEmpty', () => {
    it('should return true when no matches', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.age > 100
      );

      expect(result.isEmpty()).toBe(true);
    });

    it('should return false when has matches', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.isEmpty()).toBe(false);
    });
  });

  describe('costs', () => {
    it('should add overhead to retrieval cost', () => {
      const source = new SetResultSet(new Set(['1', '2']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.getRetrievalCost()).toBe(40); // 30 + 10
    });

    it('should estimate merge cost as half of source', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.getMergeCost()).toBe(2); // 4 / 2
    });

    it('should have minimum merge cost of 1', () => {
      const source = new SetResultSet(new Set(['1']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.getMergeCost()).toBe(1);
    });
  });

  describe('caching', () => {
    it('should cache results after first iteration', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => r.active
      );

      expect(result.isMaterialized()).toBe(false);

      const first = result.toArray();
      expect(result.isMaterialized()).toBe(true);

      const second = result.toArray();
      expect(first).toBe(second);
    });

    it('should use cache on subsequent iterations', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      let callCount = 0;
      const result = new FilteringResultSet(
        source,
        (key) => {
          callCount++;
          return getRecord(key);
        },
        (r) => r.active
      );

      // First call to toArray populates cache
      result.toArray();
      const callsAfterFirst = callCount;

      // Second call should use cache
      result.toArray();
      expect(callCount).toBe(callsAfterFirst); // No additional calls
    });
  });

  describe('lazy evaluation', () => {
    it('should not materialize until accessed', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      let filterCalled = false;
      const result = new FilteringResultSet(
        source,
        getRecord,
        (r) => {
          filterCalled = true;
          return r.active;
        }
      );

      expect(filterCalled).toBe(false);
      expect(result.isMaterialized()).toBe(false);

      // Access first element
      for (const _ of result) break;

      expect(filterCalled).toBe(true);
    });
  });
});
