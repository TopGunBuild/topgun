/**
 * UnionResultSet Tests
 */

import { UnionResultSet } from '../../../query/resultset/UnionResultSet';
import { SetResultSet } from '../../../query/resultset/SetResultSet';

describe('UnionResultSet', () => {
  describe('union logic', () => {
    it('should return union of sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.toArray().sort()).toEqual(['a', 'b', 'c', 'd']);
    });

    it('should deduplicate results', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.toArray().sort()).toEqual(['a', 'b', 'c', 'd']);
      expect(result.size()).toBe(4);
    });

    it('should handle three sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 30);
      const set3 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2, set3]);

      expect(result.toArray().sort()).toEqual(['a', 'b', 'c', 'd']);
    });

    it('should handle single set', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);

      const result = new UnionResultSet([set1]);

      expect(result.toArray()).toEqual(['a', 'b', 'c']);
    });

    it('should handle empty union', () => {
      const result = new UnionResultSet([]);

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should handle empty sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.toArray()).toEqual(['a', 'b']);
    });

    it('should handle all empty sets', () => {
      const set1 = new SetResultSet(new Set<string>(), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });
  });

  describe('contains', () => {
    it('should check membership in any set', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.contains('a')).toBe(true);
      expect(result.contains('b')).toBe(true);
      expect(result.contains('c')).toBe(true);
      expect(result.contains('d')).toBe(true);
      expect(result.contains('x')).toBe(false);
    });
  });

  describe('size', () => {
    it('should return correct size', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.size()).toBe(4);
    });

    it('should count deduplicated size', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.size()).toBe(4); // Not 6
    });
  });

  describe('isEmpty', () => {
    it('should return true when all sources empty', () => {
      const set1 = new SetResultSet(new Set<string>(), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.isEmpty()).toBe(true);
    });

    it('should return false when any source has elements', () => {
      const set1 = new SetResultSet(new Set(['a']), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.isEmpty()).toBe(false);
    });

    it('should return true for empty union', () => {
      const result = new UnionResultSet([]);

      expect(result.isEmpty()).toBe(true);
    });
  });

  describe('costs', () => {
    it('should sum retrieval costs', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 40);

      const result = new UnionResultSet([set1, set2]);

      expect(result.getRetrievalCost()).toBe(70);
    });

    it('should sum merge costs', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['d', 'e']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.getMergeCost()).toBe(5); // 3 + 2
    });

    it('should handle MAX_SAFE_INTEGER cost', () => {
      const set1 = new SetResultSet(new Set(['a']), Number.MAX_SAFE_INTEGER);
      const set2 = new SetResultSet(new Set(['b']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.getRetrievalCost()).toBe(Number.MAX_SAFE_INTEGER);
    });
  });

  describe('caching', () => {
    it('should cache results after first iteration', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      expect(result.isMaterialized()).toBe(false);

      const first = result.toArray();
      expect(result.isMaterialized()).toBe(true);

      const second = result.toArray();
      expect(first).toBe(second); // Same reference
    });

    it('should use cache on subsequent iterations', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 30);

      const result = new UnionResultSet([set1, set2]);

      // First iteration
      const first = [...result];

      // Second iteration should use cache
      const second = [...result];

      expect(first).toEqual(second);
    });
  });

  describe('iteration order', () => {
    it('should yield elements in order of sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new UnionResultSet([set1, set2]);

      const items = [...result];

      // First elements from set1
      expect(items.slice(0, 2)).toContain('a');
      expect(items.slice(0, 2)).toContain('b');

      // Then elements from set2
      expect(items.slice(2)).toContain('c');
      expect(items.slice(2)).toContain('d');
    });

    it('should skip duplicates from later sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 30);

      const result = new UnionResultSet([set1, set2]);

      const items = [...result];

      // 'b' should appear only once (from set1)
      expect(items.filter((x) => x === 'b')).toHaveLength(1);
      expect(items.indexOf('b')).toBeLessThan(2); // 'b' is from set1
    });
  });
});
