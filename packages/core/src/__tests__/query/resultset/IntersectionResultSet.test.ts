/**
 * IntersectionResultSet Tests
 */

import { IntersectionResultSet } from '../../../query/resultset/IntersectionResultSet';
import { SetResultSet } from '../../../query/resultset/SetResultSet';

describe('IntersectionResultSet', () => {
  describe('intersection logic', () => {
    it('should return intersection of sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c', 'd']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'e', 'f']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.toArray().sort()).toEqual(['b', 'c']);
    });

    it('should handle three sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c', 'd']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'e']), 30);
      const set3 = new SetResultSet(new Set(['c', 'f', 'g']), 30);

      const result = new IntersectionResultSet([set1, set2, set3]);

      expect(result.toArray()).toEqual(['c']);
    });

    it('should return empty for disjoint sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['c', 'd']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should handle single set', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);

      const result = new IntersectionResultSet([set1]);

      expect(result.toArray()).toEqual(['a', 'b', 'c']);
    });

    it('should handle empty intersection', () => {
      const result = new IntersectionResultSet([]);

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });

    it('should handle empty set', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.toArray()).toEqual([]);
    });
  });

  describe('smallest first strategy', () => {
    it('should iterate smallest set first', () => {
      // Create sets with different sizes and costs
      const largeSet = new SetResultSet(
        new Set(['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j']),
        30
      );
      const smallSet = new SetResultSet(new Set(['a', 'b', 'c']), 30);

      const result = new IntersectionResultSet([largeSet, smallSet]);

      // The smallest set (3 elements) should determine merge cost
      expect(result.getMergeCost()).toBe(3);

      // Result should only contain elements from intersection
      expect(result.toArray()).toEqual(['a', 'b', 'c']);
    });

    it('should sort by merge cost not retrieval cost', () => {
      const expensiveSmall = new SetResultSet(new Set(['a', 'b']), 100);
      const cheapLarge = new SetResultSet(
        new Set(['a', 'b', 'c', 'd', 'e', 'f']),
        10
      );

      const result = new IntersectionResultSet([cheapLarge, expensiveSmall]);

      // Merge cost should be based on smallest set (2)
      expect(result.getMergeCost()).toBe(2);
    });
  });

  describe('contains', () => {
    it('should check membership in all sets', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'd']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.contains('b')).toBe(true);
      expect(result.contains('c')).toBe(true);
      expect(result.contains('a')).toBe(false); // Only in set1
      expect(result.contains('d')).toBe(false); // Only in set2
      expect(result.contains('x')).toBe(false); // In neither
    });
  });

  describe('size', () => {
    it('should return correct size', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c', 'd']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'e']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.size()).toBe(2);
    });

    it('should return 0 for empty intersection', () => {
      const set1 = new SetResultSet(new Set(['a']), 30);
      const set2 = new SetResultSet(new Set(['b']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.size()).toBe(0);
    });
  });

  describe('isEmpty', () => {
    it('should return true for empty intersection', () => {
      const set1 = new SetResultSet(new Set(['a']), 30);
      const set2 = new SetResultSet(new Set(['b']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.isEmpty()).toBe(true);
    });

    it('should return false for non-empty intersection', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.isEmpty()).toBe(false);
    });

    it('should return true when any source is empty', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set<string>(), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.isEmpty()).toBe(true);
    });
  });

  describe('costs', () => {
    it('should return minimum retrieval cost', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 40);
      const set3 = new SetResultSet(new Set(['b']), 50);

      const result = new IntersectionResultSet([set1, set2, set3]);

      expect(result.getRetrievalCost()).toBe(30);
    });

    it('should return smallest merge cost', () => {
      const largeSet = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const smallSet = new SetResultSet(new Set(['a', 'b']), 30);

      const result = new IntersectionResultSet([largeSet, smallSet]);

      expect(result.getMergeCost()).toBe(2);
    });
  });

  describe('caching', () => {
    it('should cache results after first iteration', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'd']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.isMaterialized()).toBe(false);

      const first = result.toArray();
      expect(result.isMaterialized()).toBe(true);

      const second = result.toArray();
      expect(first).toBe(second); // Same reference
    });

    it('should use cache on subsequent iterations', () => {
      const set1 = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c', 'd']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      // First iteration
      const first = [...result];

      // Second iteration should use cache
      const second = [...result];

      expect(first).toEqual(second);
    });
  });

  describe('lazy evaluation', () => {
    it('should not materialize until accessed', () => {
      const set1 = new SetResultSet(new Set(['a', 'b']), 30);
      const set2 = new SetResultSet(new Set(['b', 'c']), 30);

      const result = new IntersectionResultSet([set1, set2]);

      expect(result.isMaterialized()).toBe(false);

      // Access size (which materializes)
      result.size();

      expect(result.isMaterialized()).toBe(true);
    });
  });
});
