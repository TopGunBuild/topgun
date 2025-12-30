/**
 * SortedResultSet Tests
 */

import { SortedResultSet, createFieldComparator } from '../../../query/resultset/SortedResultSet';
import { SetResultSet } from '../../../query/resultset/SetResultSet';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  score: number;
}

describe('SortedResultSet', () => {
  const records: Map<string, TestRecord> = new Map([
    ['1', { id: '1', name: 'Charlie', age: 35, score: 85 }],
    ['2', { id: '2', name: 'Alice', age: 30, score: 90 }],
    ['3', { id: '3', name: 'Bob', age: 25, score: 75 }],
    ['4', { id: '4', name: 'Diana', age: 28, score: 95 }],
    ['5', { id: '5', name: 'Eve', age: 40, score: 80 }],
  ]);

  const getRecord = (key: string) => records.get(key);

  describe('sorting logic', () => {
    it('should sort by field ascending', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      const sorted = result.toArray();
      const ages = sorted.map((k) => getRecord(k)!.age);

      expect(ages).toEqual([25, 28, 30, 35, 40]);
    });

    it('should sort by field descending', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'desc'
      );

      const sorted = result.toArray();
      const ages = sorted.map((k) => getRecord(k)!.age);

      expect(ages).toEqual([40, 35, 30, 28, 25]);
    });

    it('should sort by string field ascending', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'name',
        'asc'
      );

      const sorted = result.toArray();
      const names = sorted.map((k) => getRecord(k)!.name);

      expect(names).toEqual(['Alice', 'Bob', 'Charlie', 'Diana', 'Eve']);
    });

    it('should sort by string field descending', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'name',
        'desc'
      );

      const sorted = result.toArray();
      const names = sorted.map((k) => getRecord(k)!.name);

      expect(names).toEqual(['Eve', 'Diana', 'Charlie', 'Bob', 'Alice']);
    });

    it('should handle empty source', () => {
      const source = new SetResultSet(new Set<string>(), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.toArray()).toEqual([]);
    });

    it('should handle single element', () => {
      const source = new SetResultSet(new Set(['1']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.toArray()).toEqual(['1']);
    });
  });

  describe('pre-sorted handling', () => {
    it('should use pre-sorted flag to skip sort', () => {
      const source = new SetResultSet(new Set(['3', '4', '2', '1', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc',
        true // pre-sorted
      );

      // Should not sort, just pass through in order
      const items = [...result];
      expect(items).toEqual(['3', '4', '2', '1', '5']);
    });

    it('should reverse pre-sorted results for desc', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'desc',
        true // pre-sorted ascending
      );

      const items = [...result];
      expect(items).toEqual(['3', '2', '1']);
    });

    it('should report pre-sorted status', () => {
      const source = new SetResultSet(new Set(['1', '2']), 30);

      const preSorted = new SortedResultSet(source, getRecord, 'age', 'asc', true);
      expect(preSorted.isIndexSorted()).toBe(true);

      const notPreSorted = new SortedResultSet(source, getRecord, 'age', 'asc', false);
      expect(notPreSorted.isIndexSorted()).toBe(false);
    });
  });

  describe('undefined values handling', () => {
    it('should put undefined records at end for asc', () => {
      const mixedRecords: Map<string, TestRecord | undefined> = new Map([
        ['1', { id: '1', name: 'Alice', age: 30, score: 90 }],
        ['2', undefined],
        ['3', { id: '3', name: 'Bob', age: 25, score: 75 }],
      ]);

      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        (k) => mixedRecords.get(k),
        'age',
        'asc'
      );

      const sorted = result.toArray();
      expect(sorted[sorted.length - 1]).toBe('2'); // undefined at end
    });

    it('should put undefined records at start for desc', () => {
      const mixedRecords: Map<string, TestRecord | undefined> = new Map([
        ['1', { id: '1', name: 'Alice', age: 30, score: 90 }],
        ['2', undefined],
        ['3', { id: '3', name: 'Bob', age: 25, score: 75 }],
      ]);

      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        (k) => mixedRecords.get(k),
        'age',
        'desc'
      );

      const sorted = result.toArray();
      expect(sorted[0]).toBe('2'); // undefined at start
    });
  });

  describe('contains', () => {
    it('should check membership in source', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.contains('1')).toBe(true);
      expect(result.contains('4')).toBe(false);
    });
  });

  describe('size', () => {
    it('should return same size as source', () => {
      const source = new SetResultSet(new Set(['1', '2', '3', '4', '5']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.size()).toBe(5);
    });
  });

  describe('isEmpty', () => {
    it('should reflect source emptiness', () => {
      const emptySource = new SetResultSet(new Set<string>(), 30);
      const emptyResult = new SortedResultSet(
        emptySource,
        getRecord,
        'age',
        'asc'
      );
      expect(emptyResult.isEmpty()).toBe(true);

      const nonEmptySource = new SetResultSet(new Set(['1']), 30);
      const nonEmptyResult = new SortedResultSet(
        nonEmptySource,
        getRecord,
        'age',
        'asc'
      );
      expect(nonEmptyResult.isEmpty()).toBe(false);
    });
  });

  describe('costs', () => {
    it('should have minimal overhead for pre-sorted', () => {
      const source = new SetResultSet(new Set(['1', '2']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc',
        true
      );

      expect(result.getRetrievalCost()).toBe(31); // 30 + 1
    });

    it('should have higher overhead for in-memory sort', () => {
      const source = new SetResultSet(new Set(['1', '2']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc',
        false
      );

      expect(result.getRetrievalCost()).toBe(80); // 30 + 50
    });

    it('should have same merge cost as source', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.getMergeCost()).toBe(3);
    });
  });

  describe('caching', () => {
    it('should cache sorted results', () => {
      const source = new SetResultSet(new Set(['1', '2', '3']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.isMaterialized()).toBe(false);

      const first = result.toArray();
      expect(result.isMaterialized()).toBe(true);

      const second = result.toArray();
      expect(first).toBe(second);
    });
  });

  describe('metadata', () => {
    it('should return sort field', () => {
      const source = new SetResultSet(new Set(['1']), 30);
      const result = new SortedResultSet(
        source,
        getRecord,
        'age',
        'asc'
      );

      expect(result.getSortField()).toBe('age');
    });

    it('should return sort direction', () => {
      const source = new SetResultSet(new Set(['1']), 30);

      const ascResult = new SortedResultSet(source, getRecord, 'age', 'asc');
      expect(ascResult.getSortDirection()).toBe('asc');

      const descResult = new SortedResultSet(source, getRecord, 'age', 'desc');
      expect(descResult.getSortDirection()).toBe('desc');
    });
  });
});

describe('createFieldComparator', () => {
  interface SimpleRecord {
    name: string;
    age: number;
  }

  it('should create ascending comparator', () => {
    const cmp = createFieldComparator<SimpleRecord>('age', 'asc');

    expect(cmp({ name: 'A', age: 20 }, { name: 'B', age: 30 })).toBeLessThan(0);
    expect(cmp({ name: 'A', age: 30 }, { name: 'B', age: 20 })).toBeGreaterThan(0);
    expect(cmp({ name: 'A', age: 20 }, { name: 'B', age: 20 })).toBe(0);
  });

  it('should create descending comparator', () => {
    const cmp = createFieldComparator<SimpleRecord>('age', 'desc');

    expect(cmp({ name: 'A', age: 20 }, { name: 'B', age: 30 })).toBeGreaterThan(0);
    expect(cmp({ name: 'A', age: 30 }, { name: 'B', age: 20 })).toBeLessThan(0);
    expect(cmp({ name: 'A', age: 20 }, { name: 'B', age: 20 })).toBe(0);
  });

  it('should handle string fields', () => {
    const cmp = createFieldComparator<SimpleRecord>('name', 'asc');

    expect(cmp({ name: 'Alice', age: 20 }, { name: 'Bob', age: 30 })).toBeLessThan(0);
    expect(cmp({ name: 'Bob', age: 30 }, { name: 'Alice', age: 20 })).toBeGreaterThan(0);
    expect(cmp({ name: 'Alice', age: 20 }, { name: 'Alice', age: 30 })).toBe(0);
  });

  it('should handle undefined values', () => {
    const cmp = createFieldComparator<Partial<SimpleRecord>>('age', 'asc');

    expect(cmp({ name: 'A' }, { name: 'B', age: 20 })).toBeGreaterThan(0); // undefined last
    expect(cmp({ name: 'A', age: 20 }, { name: 'B' })).toBeLessThan(0);
    expect(cmp({ name: 'A' }, { name: 'B' })).toBe(0); // both undefined
  });
});
