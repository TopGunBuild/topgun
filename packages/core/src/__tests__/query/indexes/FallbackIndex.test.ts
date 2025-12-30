/**
 * FallbackIndex Tests
 */

import { FallbackIndex, createPredicateMatcher } from '../../../query/indexes/FallbackIndex';
import type { IndexQuery } from '../../../query/indexes/types';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  active: boolean;
  tags?: string[];
}

describe('FallbackIndex', () => {
  const records: Map<string, TestRecord> = new Map([
    ['1', { id: '1', name: 'Alice', age: 30, active: true }],
    ['2', { id: '2', name: 'Bob', age: 25, active: false }],
    ['3', { id: '3', name: 'Charlie', age: 35, active: true }],
  ]);

  const getAllKeys = () => records.keys();
  const getRecord = (key: string) => records.get(key);

  describe('retrieve', () => {
    it('should perform full scan with predicate', () => {
      const matchesPredicate = (record: TestRecord, query: IndexQuery<unknown>) => {
        if (query.type === 'equal') {
          return record.age === query.value;
        }
        return false;
      };

      const index = new FallbackIndex(getAllKeys, getRecord, matchesPredicate);

      const query: IndexQuery<number> & { attribute: string } = {
        type: 'equal',
        value: 30,
        attribute: 'age',
      };

      const result = index.retrieve(query);
      const keys = [...result];

      expect(keys).toEqual(['1']);
    });

    it('should return empty for no matches', () => {
      const matchesPredicate = (record: TestRecord, query: IndexQuery<unknown>) => {
        if (query.type === 'equal') {
          return record.age === query.value;
        }
        return false;
      };

      const index = new FallbackIndex(getAllKeys, getRecord, matchesPredicate);

      const query: IndexQuery<number> = { type: 'equal', value: 999 };
      const result = index.retrieve(query);

      expect([...result]).toEqual([]);
    });

    it('should support any query type', () => {
      const matchesPredicate = (record: TestRecord, query: IndexQuery<unknown>) => {
        if (query.type === 'gt') {
          return record.age > (query.value as number);
        }
        return false;
      };

      const index = new FallbackIndex(getAllKeys, getRecord, matchesPredicate);

      const query: IndexQuery<number> = { type: 'gt', value: 27 };
      const result = index.retrieve(query);

      expect([...result].sort()).toEqual(['1', '3']);
    });
  });

  describe('supportsQuery', () => {
    it('should support any query type', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      // FallbackIndex.supportsQuery() takes no arguments and always returns true
      expect(index.supportsQuery()).toBe(true);
    });
  });

  describe('getRetrievalCost', () => {
    it('should return MAX_SAFE_INTEGER', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      expect(index.getRetrievalCost()).toBe(Number.MAX_SAFE_INTEGER);
    });
  });

  describe('no-op operations', () => {
    it('should have no-op add/remove/update/clear', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      // These no-op methods take no arguments and should not throw
      expect(() => index.add()).not.toThrow();
      expect(() => index.remove()).not.toThrow();
      expect(() => index.update()).not.toThrow();
      expect(() => index.clear()).not.toThrow();
    });
  });

  describe('getStats', () => {
    it('should return empty stats', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      const stats = index.getStats();
      expect(stats).toEqual({
        distinctValues: 0,
        totalEntries: 0,
        avgEntriesPerValue: 0,
      });
    });
  });

  describe('type and attribute', () => {
    it('should have type hash', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      expect(index.type).toBe('hash');
    });

    it('should have wildcard attribute', () => {
      const index = new FallbackIndex(getAllKeys, getRecord, () => true);

      expect(index.attribute.name).toBe('*');
    });
  });
});

describe('createPredicateMatcher', () => {
  interface TestRecord {
    name: string;
    age: number;
    score: number;
    [key: string]: unknown;
  }

  const getAttribute = (record: TestRecord, attrName: string): unknown => {
    return record[attrName];
  };

  const testRecord: TestRecord = { name: 'Alice', age: 30, score: 85 };

  describe('equal query', () => {
    it('should match equal values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'equal',
        value: 30,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });

    it('should not match different values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'equal',
        value: 25,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(false);
    });
  });

  describe('in query', () => {
    it('should match values in array', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'in',
        values: [25, 30, 35],
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });

    it('should not match values not in array', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'in',
        values: [25, 35],
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(false);
    });
  });

  describe('has query', () => {
    it('should match existing attribute', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<unknown> & { attribute: string } = {
        type: 'has',
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });

    it('should not match undefined attribute', () => {
      const recordWithUndefined = { name: 'Alice', age: undefined, score: 85 } as unknown as TestRecord;
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<unknown> & { attribute: string } = {
        type: 'has',
        attribute: 'age',
      };

      expect(matcher(recordWithUndefined, query)).toBe(false);
    });
  });

  describe('gt query', () => {
    it('should match greater values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'gt',
        value: 25,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });

    it('should not match equal values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'gt',
        value: 30,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(false);
    });
  });

  describe('gte query', () => {
    it('should match greater or equal values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'gte',
        value: 30,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });
  });

  describe('lt query', () => {
    it('should match lesser values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'lt',
        value: 35,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });
  });

  describe('lte query', () => {
    it('should match lesser or equal values', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'lte',
        value: 30,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });
  });

  describe('between query', () => {
    it('should match values in range (inclusive from)', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'between',
        from: 25,
        to: 35,
        fromInclusive: true,
        toInclusive: false,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });

    it('should match with default inclusivity', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'between',
        from: 30,
        to: 35,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(true); // fromInclusive defaults to true
    });

    it('should not match outside range', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<number> & { attribute: string } = {
        type: 'between',
        from: 35,
        to: 40,
        attribute: 'age',
      };

      expect(matcher(testRecord, query)).toBe(false);
    });
  });

  describe('wildcard attribute', () => {
    it('should match everything for wildcard', () => {
      const matcher = createPredicateMatcher<TestRecord>(getAttribute);
      const query: IndexQuery<unknown> & { attribute: string } = {
        type: 'equal',
        attribute: '*',
      };

      expect(matcher(testRecord, query)).toBe(true);
    });
  });
});
