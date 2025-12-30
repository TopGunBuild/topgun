import { LazyResultSet } from '../../../query/resultset/LazyResultSet';

describe('LazyResultSet', () => {
  describe('lazy iteration', () => {
    it('should iterate lazily without full materialization', () => {
      let callCount = 0;
      const factory = function* (): Generator<string> {
        callCount++;
        yield 'a';
        yield 'b';
        yield 'c';
      };

      const rs = new LazyResultSet(factory, 40, 3);

      // Factory should not be called yet
      expect(callCount).toBe(0);
      expect(rs.isMaterialized()).toBe(false);

      // Iterate
      const result: string[] = [];
      for (const key of rs) {
        result.push(key);
      }

      expect(result).toEqual(['a', 'b', 'c']);
      // Factory was called during iteration
      expect(callCount).toBe(1);
      // Still not materialized (cached) because we didn't call toArray()
      expect(rs.isMaterialized()).toBe(false);
    });

    it('should cache after toArray()', () => {
      let callCount = 0;
      const factory = function* (): Generator<number> {
        callCount++;
        yield 1;
        yield 2;
        yield 3;
      };

      const rs = new LazyResultSet(factory, 40, 3);

      // First call to toArray
      const arr1 = rs.toArray();
      expect(arr1).toEqual([1, 2, 3]);
      expect(callCount).toBe(1);
      expect(rs.isMaterialized()).toBe(true);

      // Second call should use cache
      const arr2 = rs.toArray();
      expect(arr2).toEqual([1, 2, 3]);
      expect(callCount).toBe(1); // Factory not called again

      // Third iteration should use cache
      const arr3 = [...rs];
      expect(arr3).toEqual([1, 2, 3]);
      expect(callCount).toBe(1); // Factory still not called again
    });

    it('should iterate from cache after materialization', () => {
      let callCount = 0;
      const factory = function* (): Generator<string> {
        callCount++;
        yield 'x';
        yield 'y';
      };

      const rs = new LazyResultSet(factory, 40, 2);

      // Materialize first
      rs.toArray();
      expect(callCount).toBe(1);

      // Iterate multiple times - should use cache
      expect([...rs]).toEqual(['x', 'y']);
      expect([...rs]).toEqual(['x', 'y']);
      expect(callCount).toBe(1);
    });
  });

  describe('cost reporting', () => {
    it('should report estimated size before materialization', () => {
      const factory = function* (): Generator<number> {
        yield 1;
        yield 2;
        yield 3;
        yield 4;
        yield 5;
      };

      const rs = new LazyResultSet(factory, 40, 10); // Estimated: 10

      expect(rs.getMergeCost()).toBe(10);
      expect(rs.getRetrievalCost()).toBe(40);
      expect(rs.getEstimatedSize()).toBe(10);
    });

    it('should report actual size after materialization', () => {
      const factory = function* (): Generator<number> {
        yield 1;
        yield 2;
        yield 3;
      };

      const rs = new LazyResultSet(factory, 40, 10); // Estimated: 10, actual: 3

      // Before materialization
      expect(rs.getMergeCost()).toBe(10);

      // Materialize
      rs.toArray();

      // After materialization
      expect(rs.getMergeCost()).toBe(3);
    });
  });

  describe('isEmpty', () => {
    it('should check isEmpty without full materialization for non-empty', () => {
      let fullyIterated = false;
      const factory = function* (): Generator<string> {
        yield 'first';
        yield 'second';
        fullyIterated = true; // This should not be reached
      };

      const rs = new LazyResultSet(factory, 40, 2);

      expect(rs.isEmpty()).toBe(false);
      expect(fullyIterated).toBe(false);
      expect(rs.isMaterialized()).toBe(false);
    });

    it('should detect empty result set', () => {
      const factory = function* (): Generator<string> {
        // Empty generator
      };

      const rs = new LazyResultSet(factory, 40, 0);

      expect(rs.isEmpty()).toBe(true);
    });

    it('should use cached value for isEmpty check', () => {
      let callCount = 0;
      const factory = function* (): Generator<string> {
        callCount++;
        yield 'a';
      };

      const rs = new LazyResultSet(factory, 40, 1);

      // Materialize first
      rs.toArray();
      expect(callCount).toBe(1);

      // isEmpty should use cache
      expect(rs.isEmpty()).toBe(false);
      expect(callCount).toBe(1); // Factory not called again
    });
  });

  describe('contains', () => {
    it('should check containment (requires materialization)', () => {
      const factory = function* (): Generator<string> {
        yield 'a';
        yield 'b';
        yield 'c';
      };

      const rs = new LazyResultSet(factory, 40, 3);

      expect(rs.contains('b')).toBe(true);
      expect(rs.isMaterialized()).toBe(true);
      expect(rs.contains('x')).toBe(false);
    });
  });

  describe('size', () => {
    it('should return actual size (requires materialization)', () => {
      const factory = function* (): Generator<number> {
        yield 1;
        yield 2;
        yield 3;
        yield 4;
      };

      const rs = new LazyResultSet(factory, 40, 10); // Estimated: 10

      expect(rs.size()).toBe(4);
      expect(rs.isMaterialized()).toBe(true);
    });
  });

  describe('materialize', () => {
    it('should force materialization and return array', () => {
      const factory = function* (): Generator<string> {
        yield 'x';
        yield 'y';
        yield 'z';
      };

      const rs = new LazyResultSet(factory, 40, 3);

      expect(rs.isMaterialized()).toBe(false);
      const arr = rs.materialize();
      expect(arr).toEqual(['x', 'y', 'z']);
      expect(rs.isMaterialized()).toBe(true);
    });
  });

  describe('edge cases', () => {
    it('should handle empty result set', () => {
      const factory = function* (): Generator<never> {
        // Empty
      };

      const rs = new LazyResultSet(factory, 40, 0);

      expect(rs.isEmpty()).toBe(true);
      expect(rs.size()).toBe(0);
      expect(rs.toArray()).toEqual([]);
      expect([...rs]).toEqual([]);
    });

    it('should handle single element', () => {
      const factory = function* (): Generator<string> {
        yield 'only';
      };

      const rs = new LazyResultSet(factory, 40, 1);

      expect(rs.isEmpty()).toBe(false);
      expect(rs.toArray()).toEqual(['only']);
      expect(rs.size()).toBe(1);
    });

    it('should handle large result sets', () => {
      const factory = function* (): Generator<number> {
        for (let i = 0; i < 10000; i++) {
          yield i;
        }
      };

      const rs = new LazyResultSet(factory, 40, 10000);

      // Lazy iteration should work
      let count = 0;
      for (const _ of rs) {
        count++;
        if (count >= 100) break; // Only iterate first 100
      }

      expect(count).toBe(100);
      expect(rs.isMaterialized()).toBe(false);

      // Now materialize
      const arr = rs.toArray();
      expect(arr.length).toBe(10000);
      expect(rs.isMaterialized()).toBe(true);
    });

    it('should handle generators that throw', () => {
      const factory = function* (): Generator<number> {
        yield 1;
        yield 2;
        throw new Error('Generator error');
      };

      const rs = new LazyResultSet(factory, 40, 3);

      expect(() => rs.toArray()).toThrow('Generator error');
    });
  });

  describe('retrieval cost', () => {
    it('should report configured retrieval cost', () => {
      const factory = function* (): Generator<string> {
        yield 'a';
      };

      const rs10 = new LazyResultSet(factory, 10, 1);
      const rs40 = new LazyResultSet(factory, 40, 1);
      const rs100 = new LazyResultSet(factory, 100, 1);

      expect(rs10.getRetrievalCost()).toBe(10);
      expect(rs40.getRetrievalCost()).toBe(40);
      expect(rs100.getRetrievalCost()).toBe(100);
    });
  });
});
