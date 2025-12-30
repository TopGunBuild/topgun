/**
 * LimitResultSet Tests
 */

import { LimitResultSet } from '../../../query/resultset/LimitResultSet';
import { SetResultSet } from '../../../query/resultset/SetResultSet';

describe('LimitResultSet', () => {
  describe('limit logic', () => {
    it('should limit results', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 0, 3);

      expect(result.toArray()).toHaveLength(3);
    });

    it('should return all if limit exceeds size', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 10);

      expect(result.toArray()).toHaveLength(3);
    });

    it('should return empty if limit is 0', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 0);

      expect(result.toArray()).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });
  });

  describe('offset logic', () => {
    it('should skip offset', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 2);

      const items = result.toArray();
      expect(items).toHaveLength(3);
      expect(items).not.toContain('a');
      expect(items).not.toContain('b');
    });

    it('should return empty if offset exceeds size', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 10);

      expect(result.toArray()).toEqual([]);
    });
  });

  describe('offset and limit together', () => {
    it('should combine offset and limit', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 1, 2);

      const items = result.toArray();
      expect(items).toHaveLength(2);
      // First item 'a' skipped, next 2 returned
    });

    it('should handle offset at end with small limit', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 4, 10);

      const items = result.toArray();
      expect(items).toHaveLength(1); // Only 'e' remains after offset of 4
    });

    it('should handle pagination', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j']), 30);

      // Page 1: offset=0, limit=3
      const page1 = new LimitResultSet(source, 0, 3);
      expect(page1.toArray()).toHaveLength(3);

      // Page 2: offset=3, limit=3
      const page2 = new LimitResultSet(source, 3, 3);
      expect(page2.toArray()).toHaveLength(3);

      // Page 3: offset=6, limit=3
      const page3 = new LimitResultSet(source, 6, 3);
      expect(page3.toArray()).toHaveLength(3);

      // Page 4: offset=9, limit=3
      const page4 = new LimitResultSet(source, 9, 3);
      expect(page4.toArray()).toHaveLength(1); // Only 1 item left
    });
  });

  describe('early termination', () => {
    it('should terminate early when limit reached', () => {
      let iterationCount = 0;
      const customIterator = function* () {
        for (const item of ['a', 'b', 'c', 'd', 'e']) {
          iterationCount++;
          yield item;
        }
      };

      const source = {
        [Symbol.iterator]: customIterator,
        getRetrievalCost: () => 30,
        getMergeCost: () => 5,
        contains: (k: string) => ['a', 'b', 'c', 'd', 'e'].includes(k),
        size: () => 5,
        toArray: () => ['a', 'b', 'c', 'd', 'e'],
        isEmpty: () => false,
      };

      const result = new LimitResultSet(source, 0, 2);

      [...result]; // Trigger iteration

      // For-of fetches next item before break condition is checked,
      // so we iterate limit + 1 items (3rd item fetched before break)
      expect(iterationCount).toBe(3);
    });

    it('should not iterate all for offset + limit', () => {
      let iterationCount = 0;
      const customIterator = function* () {
        for (const item of ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']) {
          iterationCount++;
          yield item;
        }
      };

      const source = {
        [Symbol.iterator]: customIterator,
        getRetrievalCost: () => 30,
        getMergeCost: () => 8,
        contains: (k: string) => ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'].includes(k),
        size: () => 8,
        toArray: () => ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'],
        isEmpty: () => false,
      };

      const result = new LimitResultSet(source, 2, 3);

      [...result]; // Trigger iteration

      // For-of fetches next item before break, so: 2 skipped + 3 returned + 1 for break check
      expect(iterationCount).toBe(6);
    });
  });

  describe('no limit/offset', () => {
    it('should pass through with no restrictions', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, Infinity);

      expect(result.toArray()).toEqual(['a', 'b', 'c']);
    });

    it('should use defaults', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source);

      expect(result.toArray()).toEqual(['a', 'b', 'c']);
      expect(result.getOffset()).toBe(0);
      expect(result.getLimit()).toBe(Infinity);
    });
  });

  describe('contains', () => {
    it('should check if in limited result', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 0, 3);

      // First 3 items should be contained
      const items = result.toArray();
      for (const item of items) {
        expect(result.contains(item)).toBe(true);
      }

      // Item 'd' and 'e' are beyond limit
      // Note: this requires materialization to check position
    });

    it('should return false for items not in source', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 2);

      expect(result.contains('x')).toBe(false);
    });
  });

  describe('size', () => {
    it('should return limited size', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 0, 3);

      expect(result.size()).toBe(3);
    });

    it('should return size after offset', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 3);

      expect(result.size()).toBe(2);
    });

    it('should return offset + limit constrained size', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);
      const result = new LimitResultSet(source, 2, 2);

      expect(result.size()).toBe(2);
    });
  });

  describe('isEmpty', () => {
    it('should return true when limit is 0', () => {
      const source = new SetResultSet(new Set(['a', 'b']), 30);
      const result = new LimitResultSet(source, 0, 0);

      expect(result.isEmpty()).toBe(true);
    });

    it('should return true when offset exceeds size', () => {
      const source = new SetResultSet(new Set(['a', 'b']), 30);
      const result = new LimitResultSet(source, 10);

      expect(result.isEmpty()).toBe(true);
    });

    it('should return false when has results', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 2);

      expect(result.isEmpty()).toBe(false);
    });
  });

  describe('costs', () => {
    it('should return same retrieval cost as source', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 2);

      expect(result.getRetrievalCost()).toBe(30);
    });

    it('should estimate merge cost as min(source, offset + limit)', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c', 'd', 'e']), 30);

      const limited = new LimitResultSet(source, 0, 2);
      expect(limited.getMergeCost()).toBe(2);

      const allLimited = new LimitResultSet(source, 0, 10);
      expect(allLimited.getMergeCost()).toBe(5); // source size

      const withOffset = new LimitResultSet(source, 2, 2);
      expect(withOffset.getMergeCost()).toBe(4); // offset + limit
    });
  });

  describe('caching', () => {
    it('should cache results after iteration', () => {
      const source = new SetResultSet(new Set(['a', 'b', 'c']), 30);
      const result = new LimitResultSet(source, 0, 2);

      expect(result.isMaterialized()).toBe(false);

      const first = result.toArray();
      expect(result.isMaterialized()).toBe(true);

      const second = result.toArray();
      expect(first).toBe(second);
    });
  });

  describe('metadata', () => {
    it('should return offset', () => {
      const source = new SetResultSet(new Set(['a', 'b']), 30);
      const result = new LimitResultSet(source, 5, 10);

      expect(result.getOffset()).toBe(5);
    });

    it('should return limit', () => {
      const source = new SetResultSet(new Set(['a', 'b']), 30);
      const result = new LimitResultSet(source, 0, 10);

      expect(result.getLimit()).toBe(10);
    });
  });
});
