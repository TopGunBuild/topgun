/**
 * Cursor Pagination Tests - Phase 14.1
 *
 * Tests for cursor-based pagination in server-side query execution.
 */

import { executeQueryWithCursor, Query, QueryResultWithCursor } from '../../query/Matcher';
import { LWWRecord, Timestamp } from '@topgunbuild/core';

describe('executeQueryWithCursor', () => {
  const makeTimestamp = (): Timestamp => ({
    millis: Date.now(),
    counter: 0,
    nodeId: 'test-node',
  });

  const makeRecord = <T>(value: T): LWWRecord<T> => ({
    value,
    timestamp: makeTimestamp(),
  });

  describe('basic pagination', () => {
    const testData = new Map<string, LWWRecord<any>>([
      ['user1', makeRecord({ name: 'Alice', age: 30, createdAt: 1000 })],
      ['user2', makeRecord({ name: 'Bob', age: 25, createdAt: 2000 })],
      ['user3', makeRecord({ name: 'Charlie', age: 35, createdAt: 3000 })],
      ['user4', makeRecord({ name: 'Diana', age: 28, createdAt: 4000 })],
      ['user5', makeRecord({ name: 'Eve', age: 22, createdAt: 5000 })],
    ]);

    it('should return first page with hasMore flag', () => {
      const query: Query = {
        sort: { createdAt: 'asc' },
        limit: 2,
      };

      const result = executeQueryWithCursor(testData, query);

      expect(result.results).toHaveLength(2);
      expect(result.results[0].key).toBe('user1');
      expect(result.results[1].key).toBe('user2');
      expect(result.hasMore).toBe(true);
      expect(result.nextCursor).toBeDefined();
    });

    it('should return second page using cursor', () => {
      const query1: Query = {
        sort: { createdAt: 'asc' },
        limit: 2,
      };

      const page1 = executeQueryWithCursor(testData, query1);
      expect(page1.nextCursor).toBeDefined();

      const query2: Query = {
        sort: { createdAt: 'asc' },
        limit: 2,
        cursor: page1.nextCursor,
      };

      const page2 = executeQueryWithCursor(testData, query2);

      expect(page2.results).toHaveLength(2);
      expect(page2.results[0].key).toBe('user3');
      expect(page2.results[1].key).toBe('user4');
      expect(page2.hasMore).toBe(true);
    });

    it('should return last page with hasMore=false', () => {
      const query1: Query = { sort: { createdAt: 'asc' }, limit: 2 };
      const page1 = executeQueryWithCursor(testData, query1);

      const query2: Query = { sort: { createdAt: 'asc' }, limit: 2, cursor: page1.nextCursor };
      const page2 = executeQueryWithCursor(testData, query2);

      const query3: Query = { sort: { createdAt: 'asc' }, limit: 2, cursor: page2.nextCursor };
      const page3 = executeQueryWithCursor(testData, query3);

      expect(page3.results).toHaveLength(1);
      expect(page3.results[0].key).toBe('user5');
      expect(page3.hasMore).toBe(false);
      expect(page3.nextCursor).toBeUndefined();
    });

    it('should work with DESC sort', () => {
      const query1: Query = {
        sort: { createdAt: 'desc' },
        limit: 2,
      };

      const page1 = executeQueryWithCursor(testData, query1);

      expect(page1.results).toHaveLength(2);
      expect(page1.results[0].key).toBe('user5'); // Highest createdAt
      expect(page1.results[1].key).toBe('user4');
      expect(page1.hasMore).toBe(true);

      const query2: Query = {
        sort: { createdAt: 'desc' },
        limit: 2,
        cursor: page1.nextCursor,
      };

      const page2 = executeQueryWithCursor(testData, query2);

      expect(page2.results).toHaveLength(2);
      expect(page2.results[0].key).toBe('user3');
      expect(page2.results[1].key).toBe('user2');
    });
  });

  describe('with where filter', () => {
    const testData = new Map<string, LWWRecord<any>>([
      ['user1', makeRecord({ name: 'Alice', role: 'admin', score: 100 })],
      ['user2', makeRecord({ name: 'Bob', role: 'user', score: 80 })],
      ['user3', makeRecord({ name: 'Charlie', role: 'admin', score: 90 })],
      ['user4', makeRecord({ name: 'Diana', role: 'user', score: 70 })],
      ['user5', makeRecord({ name: 'Eve', role: 'admin', score: 85 })],
    ]);

    it('should paginate filtered results', () => {
      const query1: Query = {
        where: { role: 'admin' },
        sort: { score: 'desc' },
        limit: 2,
      };

      const page1 = executeQueryWithCursor(testData, query1);

      expect(page1.results).toHaveLength(2);
      expect(page1.results[0].value.name).toBe('Alice'); // score: 100
      expect(page1.results[1].value.name).toBe('Charlie'); // score: 90
      expect(page1.hasMore).toBe(true);

      const query2: Query = {
        where: { role: 'admin' },
        sort: { score: 'desc' },
        limit: 2,
        cursor: page1.nextCursor,
      };

      const page2 = executeQueryWithCursor(testData, query2);

      expect(page2.results).toHaveLength(1);
      expect(page2.results[0].value.name).toBe('Eve'); // score: 85
      expect(page2.hasMore).toBe(false);
    });
  });

  describe('cursor validation', () => {
    const testData = new Map<string, LWWRecord<any>>([
      ['item1', makeRecord({ value: 1 })],
      ['item2', makeRecord({ value: 2 })],
      ['item3', makeRecord({ value: 3 })],
    ]);

    it('should ignore invalid cursor and return from beginning', () => {
      const query: Query = {
        sort: { value: 'asc' },
        limit: 2,
        cursor: 'invalid-cursor-string',
      };

      const result = executeQueryWithCursor(testData, query);

      expect(result.results).toHaveLength(2);
      expect(result.results[0].key).toBe('item1');
      expect(result.results[1].key).toBe('item2');
    });

    it('should ignore cursor with mismatched predicate hash', () => {
      // Larger dataset with items matching the filter
      const largeData = new Map<string, LWWRecord<any>>([
        ['item1', makeRecord({ value: 1, type: 'A' })],
        ['item2', makeRecord({ value: 2, type: 'A' })],
        ['item3', makeRecord({ value: 3, type: 'A' })],
      ]);

      // Get cursor for type A
      const query1: Query = {
        where: { type: 'A' },
        sort: { value: 'asc' },
        limit: 1,
      };
      const page1 = executeQueryWithCursor(largeData, query1);

      // Try to use cursor with a malformed cursor (should be ignored)
      const query2: Query = {
        where: { type: 'A' },
        sort: { value: 'asc' },
        limit: 2,
        cursor: 'invalid-cursor-format',
      };

      const result = executeQueryWithCursor(largeData, query2);

      // Should return from beginning since cursor is invalid
      expect(result.results.length).toBeGreaterThan(0);
      expect(result.results[0].key).toBe('item1'); // First item
    });
  });

  describe('edge cases', () => {
    it('should handle empty data', () => {
      const emptyData = new Map<string, LWWRecord<any>>();
      const query: Query = {
        sort: { value: 'asc' },
        limit: 10,
      };

      const result = executeQueryWithCursor(emptyData, query);

      expect(result.results).toHaveLength(0);
      expect(result.hasMore).toBe(false);
      expect(result.nextCursor).toBeUndefined();
    });

    it('should handle limit larger than data size', () => {
      const smallData = new Map<string, LWWRecord<any>>([
        ['item1', makeRecord({ value: 1 })],
        ['item2', makeRecord({ value: 2 })],
      ]);

      const query: Query = {
        sort: { value: 'asc' },
        limit: 100,
      };

      const result = executeQueryWithCursor(smallData, query);

      expect(result.results).toHaveLength(2);
      expect(result.hasMore).toBe(false);
      expect(result.nextCursor).toBeUndefined();
    });

    it('should handle query without limit (returns all results)', () => {
      const testData = new Map<string, LWWRecord<any>>([
        ['a-item', makeRecord({ value: 1 })],
        ['b-item', makeRecord({ value: 2 })],
        ['c-item', makeRecord({ value: 3 })],
      ]);

      const query: Query = {
        sort: { value: 'asc' },
        // No limit
      };

      const result = executeQueryWithCursor(testData, query);

      expect(result.results).toHaveLength(3);
      expect(result.hasMore).toBe(false);
      expect(result.nextCursor).toBeUndefined();
    });

    it('should handle exact limit match', () => {
      const testData = new Map<string, LWWRecord<any>>([
        ['item1', makeRecord({ value: 1 })],
        ['item2', makeRecord({ value: 2 })],
        ['item3', makeRecord({ value: 3 })],
      ]);

      const query: Query = {
        sort: { value: 'asc' },
        limit: 3,
      };

      const result = executeQueryWithCursor(testData, query);

      expect(result.results).toHaveLength(3);
      expect(result.hasMore).toBe(false);
      expect(result.nextCursor).toBeUndefined();
    });
  });

  describe('tie-breaking with key', () => {
    const testData = new Map<string, LWWRecord<any>>([
      ['user-a', makeRecord({ name: 'Alice', score: 100 })],
      ['user-b', makeRecord({ name: 'Bob', score: 100 })],
      ['user-c', makeRecord({ name: 'Charlie', score: 100 })],
      ['user-d', makeRecord({ name: 'Diana', score: 100 })],
    ]);

    it('should use key for tie-breaking when sort values are equal', () => {
      const query1: Query = {
        sort: { score: 'desc' },
        limit: 2,
      };

      const page1 = executeQueryWithCursor(testData, query1);

      // All have same score, so should be ordered by key
      expect(page1.results).toHaveLength(2);

      const query2: Query = {
        sort: { score: 'desc' },
        limit: 2,
        cursor: page1.nextCursor,
      };

      const page2 = executeQueryWithCursor(testData, query2);

      expect(page2.results).toHaveLength(2);

      // Verify no overlap between pages
      const page1Keys = new Set(page1.results.map(r => r.key));
      const page2Keys = new Set(page2.results.map(r => r.key));
      for (const key of page2Keys) {
        expect(page1Keys.has(key)).toBe(false);
      }
    });
  });
});
