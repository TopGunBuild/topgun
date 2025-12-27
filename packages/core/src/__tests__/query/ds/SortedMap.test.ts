import {
  SortedMap,
  defaultComparator,
  numericComparator,
  stringComparator,
  reverseComparator,
} from '../../../query/ds';

describe('SortedMap', () => {
  describe('basic operations', () => {
    it('should set and get values', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');
      map.set(2, 'two');
      map.set(3, 'three');

      expect(map.get(1)).toBe('one');
      expect(map.get(2)).toBe('two');
      expect(map.get(3)).toBe('three');
      expect(map.get(4)).toBeUndefined();
    });

    it('should update existing keys', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');
      expect(map.get(1)).toBe('one');

      map.set(1, 'ONE');
      expect(map.get(1)).toBe('ONE');
      expect(map.size).toBe(1);
    });

    it('should delete keys', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');
      map.set(2, 'two');

      expect(map.delete(1)).toBe(true);
      expect(map.get(1)).toBeUndefined();
      expect(map.has(1)).toBe(false);
      expect(map.size).toBe(1);

      expect(map.delete(99)).toBe(false);
    });

    it('should report correct size', () => {
      const map = new SortedMap<number, string>();
      expect(map.size).toBe(0);
      expect(map.isEmpty).toBe(true);

      map.set(1, 'one');
      expect(map.size).toBe(1);
      expect(map.isEmpty).toBe(false);

      map.set(2, 'two');
      expect(map.size).toBe(2);

      map.delete(1);
      expect(map.size).toBe(1);

      map.clear();
      expect(map.size).toBe(0);
      expect(map.isEmpty).toBe(true);
    });

    it('should check key existence with has()', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');

      expect(map.has(1)).toBe(true);
      expect(map.has(2)).toBe(false);
    });

    it('should support method chaining for set', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one').set(2, 'two').set(3, 'three');

      expect(map.size).toBe(3);
    });
  });

  describe('ordering', () => {
    it('should iterate in sorted order', () => {
      const map = new SortedMap<number, string>();
      map.set(3, 'three');
      map.set(1, 'one');
      map.set(2, 'two');
      map.set(5, 'five');
      map.set(4, 'four');

      const keys = [...map.keys()];
      expect(keys).toEqual([1, 2, 3, 4, 5]);

      const values = [...map.values()];
      expect(values).toEqual(['one', 'two', 'three', 'four', 'five']);
    });

    it('should work with numeric keys', () => {
      const map = new SortedMap<number, string>(numericComparator);
      map.set(100, 'hundred');
      map.set(10, 'ten');
      map.set(1, 'one');

      const keys = [...map.keys()];
      expect(keys).toEqual([1, 10, 100]);
    });

    it('should work with string keys', () => {
      const map = new SortedMap<string, number>(stringComparator);
      map.set('banana', 2);
      map.set('apple', 1);
      map.set('cherry', 3);

      const keys = [...map.keys()];
      expect(keys).toEqual(['apple', 'banana', 'cherry']);
    });

    it('should work with custom comparator', () => {
      // Reverse order comparator
      const map = new SortedMap<number, string>(reverseComparator(numericComparator));
      map.set(1, 'one');
      map.set(2, 'two');
      map.set(3, 'three');

      const keys = [...map.keys()];
      expect(keys).toEqual([3, 2, 1]);
    });

    it('should iterate in reverse order with entriesReversed()', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');
      map.set(2, 'two');
      map.set(3, 'three');

      const entries = [...map.entriesReversed()];
      expect(entries).toEqual([
        [3, 'three'],
        [2, 'two'],
        [1, 'one'],
      ]);
    });
  });

  describe('range queries', () => {
    let map: SortedMap<number, string>;

    beforeEach(() => {
      map = new SortedMap<number, string>();
      for (let i = 1; i <= 10; i++) {
        map.set(i, `value-${i}`);
      }
    });

    it('should return range [from, to) by default', () => {
      const results = [...map.range(3, 7)];
      expect(results).toEqual([
        [3, 'value-3'],
        [4, 'value-4'],
        [5, 'value-5'],
        [6, 'value-6'],
      ]);
    });

    it('should support inclusive from (default)', () => {
      const results = [...map.range(3, 5)];
      expect(results.map(([k]) => k)).toEqual([3, 4]);
    });

    it('should support exclusive from', () => {
      const results = [...map.range(3, 7, { fromInclusive: false })];
      expect(results.map(([k]) => k)).toEqual([4, 5, 6]);
    });

    it('should support inclusive to', () => {
      const results = [...map.range(3, 7, { toInclusive: true })];
      expect(results.map(([k]) => k)).toEqual([3, 4, 5, 6, 7]);
    });

    it('should support exclusive from and inclusive to', () => {
      const results = [...map.range(3, 7, { fromInclusive: false, toInclusive: true })];
      expect(results.map(([k]) => k)).toEqual([4, 5, 6, 7]);
    });

    it('should return empty for invalid range (from > to)', () => {
      const results = [...map.range(7, 3)];
      expect(results).toEqual([]);
    });

    it('should handle range at boundaries', () => {
      const results = [...map.range(1, 10, { toInclusive: true })];
      expect(results.length).toBe(10);
    });

    it('should handle range beyond boundaries', () => {
      const results = [...map.range(0, 100, { toInclusive: true })];
      expect(results.length).toBe(10);
    });

    it('should handle single element range', () => {
      const results = [...map.range(5, 5, { toInclusive: true })];
      expect(results).toEqual([[5, 'value-5']]);
    });
  });

  describe('greaterThan', () => {
    let map: SortedMap<number, string>;

    beforeEach(() => {
      map = new SortedMap<number, string>();
      for (let i = 1; i <= 5; i++) {
        map.set(i, `value-${i}`);
      }
    });

    it('should return all keys > value (exclusive)', () => {
      const results = [...map.greaterThan(3)];
      expect(results.map(([k]) => k)).toEqual([4, 5]);
    });

    it('should return all keys >= value when inclusive', () => {
      const results = [...map.greaterThan(3, true)];
      expect(results.map(([k]) => k)).toEqual([3, 4, 5]);
    });

    it('should handle edge case: key at start', () => {
      const results = [...map.greaterThan(1)];
      expect(results.map(([k]) => k)).toEqual([2, 3, 4, 5]);
    });

    it('should handle edge case: key at end', () => {
      const results = [...map.greaterThan(5)];
      expect(results).toEqual([]);
    });

    it('should handle edge case: key beyond end', () => {
      const results = [...map.greaterThan(100)];
      expect(results).toEqual([]);
    });

    it('should handle edge case: key before start', () => {
      const results = [...map.greaterThan(0)];
      expect(results.map(([k]) => k)).toEqual([1, 2, 3, 4, 5]);
    });
  });

  describe('lessThan', () => {
    let map: SortedMap<number, string>;

    beforeEach(() => {
      map = new SortedMap<number, string>();
      for (let i = 1; i <= 5; i++) {
        map.set(i, `value-${i}`);
      }
    });

    it('should return all keys < value (exclusive)', () => {
      const results = [...map.lessThan(4)];
      expect(results.map(([k]) => k)).toEqual([1, 2, 3]);
    });

    it('should return all keys <= value when inclusive', () => {
      const results = [...map.lessThan(4, true)];
      expect(results.map(([k]) => k)).toEqual([1, 2, 3, 4]);
    });

    it('should handle edge case: key at start', () => {
      const results = [...map.lessThan(1)];
      expect(results).toEqual([]);
    });

    it('should handle edge case: key at end', () => {
      const results = [...map.lessThan(5)];
      expect(results.map(([k]) => k)).toEqual([1, 2, 3, 4]);
    });

    it('should handle edge case: key beyond end', () => {
      const results = [...map.lessThan(100)];
      expect(results.map(([k]) => k)).toEqual([1, 2, 3, 4, 5]);
    });

    it('should handle edge case: key before start', () => {
      const results = [...map.lessThan(0)];
      expect(results).toEqual([]);
    });
  });

  describe('min/max keys', () => {
    it('should return min and max keys', () => {
      const map = new SortedMap<number, string>();
      map.set(5, 'five');
      map.set(1, 'one');
      map.set(10, 'ten');

      expect(map.minKey()).toBe(1);
      expect(map.maxKey()).toBe(10);
    });

    it('should return undefined for empty map', () => {
      const map = new SortedMap<number, string>();
      expect(map.minKey()).toBeUndefined();
      expect(map.maxKey()).toBeUndefined();
    });
  });

  describe('floor/ceiling/lower/higher keys', () => {
    let map: SortedMap<number, string>;

    beforeEach(() => {
      map = new SortedMap<number, string>();
      map.set(2, 'two');
      map.set(4, 'four');
      map.set(6, 'six');
      map.set(8, 'eight');
    });

    it('should find lower key (greatest key less than)', () => {
      expect(map.lowerKey(5)).toBe(4);
      expect(map.lowerKey(6)).toBe(4);
      expect(map.lowerKey(2)).toBeUndefined();
      expect(map.lowerKey(1)).toBeUndefined();
    });

    it('should find floor key (greatest key less than or equal)', () => {
      expect(map.floorKey(5)).toBe(4);
      expect(map.floorKey(6)).toBe(6);
      expect(map.floorKey(1)).toBeUndefined();
    });

    it('should find higher key (least key greater than)', () => {
      expect(map.higherKey(5)).toBe(6);
      expect(map.higherKey(6)).toBe(8);
      expect(map.higherKey(8)).toBeUndefined();
      expect(map.higherKey(10)).toBeUndefined();
    });

    it('should find ceiling key (least key greater than or equal)', () => {
      expect(map.ceilingKey(5)).toBe(6);
      expect(map.ceilingKey(6)).toBe(6);
      expect(map.ceilingKey(9)).toBeUndefined();
    });
  });

  describe('utility methods', () => {
    it('should get entry at index', () => {
      const map = new SortedMap<number, string>();
      map.set(1, 'one');
      map.set(2, 'two');
      map.set(3, 'three');

      expect(map.at(0)).toEqual([1, 'one']);
      expect(map.at(1)).toEqual([2, 'two']);
      expect(map.at(2)).toEqual([3, 'three']);
      expect(map.at(3)).toBeUndefined();
      expect(map.at(-1)).toBeUndefined();
    });

    it('should getOrSet value with factory', () => {
      const map = new SortedMap<string, number>();
      const factory = jest.fn(() => 42);

      const value1 = map.getOrSet('key', factory);
      expect(value1).toBe(42);
      expect(factory).toHaveBeenCalledTimes(1);

      const value2 = map.getOrSet('key', factory);
      expect(value2).toBe(42);
      expect(factory).toHaveBeenCalledTimes(1); // Not called again
    });

    it('should update existing value', () => {
      const map = new SortedMap<string, number>();
      map.set('count', 1);

      const updated = map.update('count', (v) => v + 1);
      expect(updated).toBe(true);
      expect(map.get('count')).toBe(2);

      const notUpdated = map.update('missing', (v) => v + 1);
      expect(notUpdated).toBe(false);
    });

    it('should forEach in order', () => {
      const map = new SortedMap<number, string>();
      map.set(3, 'three');
      map.set(1, 'one');
      map.set(2, 'two');

      const visited: Array<[number, string]> = [];
      map.forEach((value, key) => {
        visited.push([key, value]);
      });

      expect(visited).toEqual([
        [1, 'one'],
        [2, 'two'],
        [3, 'three'],
      ]);
    });

    it('should create from entries', () => {
      const entries: Array<[number, string]> = [
        [3, 'three'],
        [1, 'one'],
        [2, 'two'],
      ];

      const map = SortedMap.from(entries);
      expect(map.size).toBe(3);
      expect([...map.keys()]).toEqual([1, 2, 3]);
    });

    it('should be iterable with for...of', () => {
      const map = new SortedMap<number, string>();
      map.set(2, 'two');
      map.set(1, 'one');

      const entries: Array<[number, string]> = [];
      for (const entry of map) {
        entries.push(entry);
      }

      expect(entries).toEqual([
        [1, 'one'],
        [2, 'two'],
      ]);
    });
  });

  describe('performance', () => {
    it('should handle 100k insertions efficiently', () => {
      const map = new SortedMap<number, string>();
      const start = performance.now();

      for (let i = 0; i < 100000; i++) {
        map.set(i, `value-${i}`);
      }

      const elapsed = performance.now() - start;
      expect(map.size).toBe(100000);
      // Should complete in reasonable time (< 5 seconds even on slow machines)
      expect(elapsed).toBeLessThan(5000);
    });

    it('should maintain O(log N) lookup time', () => {
      const map = new SortedMap<number, string>();

      // Insert 100k entries
      for (let i = 0; i < 100000; i++) {
        map.set(i, `value-${i}`);
      }

      // Perform 10k lookups
      const start = performance.now();
      for (let i = 0; i < 10000; i++) {
        const key = Math.floor(Math.random() * 100000);
        map.get(key);
      }
      const elapsed = performance.now() - start;

      // 10k lookups should be very fast (< 100ms)
      expect(elapsed).toBeLessThan(100);
    });

    it('should handle range queries efficiently', () => {
      const map = new SortedMap<number, string>();

      // Insert 100k entries
      for (let i = 0; i < 100000; i++) {
        map.set(i, `value-${i}`);
      }

      // Range query for 1000 elements
      const start = performance.now();
      const results = [...map.range(40000, 41000)];
      const elapsed = performance.now() - start;

      expect(results.length).toBe(1000);
      // Range query should be fast (< 50ms)
      expect(elapsed).toBeLessThan(50);
    });
  });

  describe('edge cases', () => {
    it('should handle empty map operations gracefully', () => {
      const map = new SortedMap<number, string>();

      expect(map.get(1)).toBeUndefined();
      expect(map.delete(1)).toBe(false);
      expect(map.has(1)).toBe(false);
      expect([...map.range(1, 10)]).toEqual([]);
      expect([...map.greaterThan(5)]).toEqual([]);
      expect([...map.lessThan(5)]).toEqual([]);
    });

    it('should handle single element map', () => {
      const map = new SortedMap<number, string>();
      map.set(5, 'five');

      expect([...map.range(1, 10)]).toEqual([[5, 'five']]);
      expect([...map.greaterThan(4)]).toEqual([[5, 'five']]);
      expect([...map.greaterThan(5)]).toEqual([]);
      expect([...map.lessThan(6)]).toEqual([[5, 'five']]);
      expect([...map.lessThan(5)]).toEqual([]);
    });

    it('should handle negative numbers', () => {
      const map = new SortedMap<number, string>();
      map.set(-5, 'minus-five');
      map.set(0, 'zero');
      map.set(5, 'five');

      expect([...map.keys()]).toEqual([-5, 0, 5]);
      expect([...map.range(-10, 10, { toInclusive: true })].map(([k]) => k)).toEqual([
        -5, 0, 5,
      ]);
    });

    it('should handle floating point numbers', () => {
      const map = new SortedMap<number, string>();
      map.set(1.5, 'one-point-five');
      map.set(1.1, 'one-point-one');
      map.set(1.9, 'one-point-nine');

      expect([...map.keys()]).toEqual([1.1, 1.5, 1.9]);
    });
  });
});

describe('Comparator utilities', () => {
  it('defaultComparator should order correctly', () => {
    expect(defaultComparator(1, 2)).toBeLessThan(0);
    expect(defaultComparator(2, 1)).toBeGreaterThan(0);
    expect(defaultComparator(1, 1)).toBe(0);
    expect(defaultComparator('a', 'b')).toBeLessThan(0);
  });

  it('numericComparator should order numbers correctly', () => {
    expect(numericComparator(1, 10)).toBeLessThan(0);
    expect(numericComparator(10, 1)).toBeGreaterThan(0);
    expect(numericComparator(5, 5)).toBe(0);
  });

  it('stringComparator should use locale comparison', () => {
    expect(stringComparator('apple', 'banana')).toBeLessThan(0);
    expect(stringComparator('banana', 'apple')).toBeGreaterThan(0);
    expect(stringComparator('apple', 'apple')).toBe(0);
  });

  it('reverseComparator should invert order', () => {
    const reversed = reverseComparator(numericComparator);
    expect(reversed(1, 2)).toBeGreaterThan(0);
    expect(reversed(2, 1)).toBeLessThan(0);
    // Note: -0 === 0 in JavaScript, so we check for 0 value
    expect(reversed(1, 1) === 0).toBe(true);
  });
});
