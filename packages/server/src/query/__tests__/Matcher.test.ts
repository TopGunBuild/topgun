import { matchesQuery, executeQuery, Query } from '../Matcher';
import { LWWRecord, Predicates } from '@topgunbuild/core';

const createRecord = (value: any): LWWRecord<any> => ({
  value,
  timestamp: { millis: Date.now(), counter: 0, nodeId: 'test' }
});

describe('Predicate Matcher', () => {
  test('should match simple equality', () => {
    const record = createRecord({ status: 'active' });
    const query: Query = { predicate: Predicates.equal('status', 'active') };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should fail simple equality mismatch', () => {
    const record = createRecord({ status: 'inactive' });
    const query: Query = { predicate: Predicates.equal('status', 'active') };
    expect(matchesQuery(record, query)).toBe(false);
  });

  test('should match AND condition', () => {
    const record = createRecord({ age: 25, active: true });
    const query: Query = {
      predicate: Predicates.and(
        Predicates.greaterThan('age', 20),
        Predicates.equal('active', true)
      )
    };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should fail AND condition', () => {
    const record = createRecord({ age: 15, active: true });
    const query: Query = {
      predicate: Predicates.and(
        Predicates.greaterThan('age', 20),
        Predicates.equal('active', true)
      )
    };
    expect(matchesQuery(record, query)).toBe(false);
  });

  test('should match OR condition', () => {
    const record = createRecord({ age: 15 });
    const query: Query = {
      predicate: Predicates.or(
        Predicates.greaterThan('age', 20),
        Predicates.lessThan('age', 18)
      )
    };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should match Regex', () => {
    const record = createRecord({ email: 'test@example.com' });
    const query: Query = {
      predicate: Predicates.regex('email', '.*@example\\.com')
    };
    expect(matchesQuery(record, query)).toBe(true);
  });
});

describe('Query Matcher (Legacy)', () => {
  test('should match exact values', () => {
    const record = createRecord({ status: 'active', priority: 1 });
    const query: Query = { where: { status: 'active' } };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should fail mismatching values', () => {
    const record = createRecord({ status: 'completed' });
    const query: Query = { where: { status: 'active' } };
    expect(matchesQuery(record, query)).toBe(false);
  });

  test('should match multiple fields (AND logic)', () => {
    const record = createRecord({ status: 'active', priority: 1 });
    const query: Query = { where: { status: 'active', priority: 1 } };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should match greater than ($gt)', () => {
    const record = createRecord({ age: 20 });
    const query: Query = { where: { age: { $gt: 18 } } };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should fail greater than ($gt)', () => {
    const record = createRecord({ age: 10 });
    const query: Query = { where: { age: { $gt: 18 } } };
    expect(matchesQuery(record, query)).toBe(false);
  });

  test('should match complex logic ($gte, $lt)', () => {
    const record = createRecord({ age: 20 });
    const query: Query = { where: { age: { $gte: 20, $lt: 30 } } };
    expect(matchesQuery(record, query)).toBe(true);
  });

  test('should handle empty query (match all)', () => {
    const record = createRecord({ any: 'thing' });
    expect(matchesQuery(record, {})).toBe(true);
  });

  test('should handle null value (deleted record)', () => {
    const record = createRecord(null);
    const query: Query = { where: { status: 'active' } };
    expect(matchesQuery(record, query)).toBe(false);
  });
});

describe('executeQuery', () => {
  const records = new Map<string, LWWRecord<any>>();
  
  beforeAll(() => {
    records.set('1', createRecord({ name: 'A', age: 10 }));
    records.set('2', createRecord({ name: 'B', age: 20 }));
    records.set('3', createRecord({ name: 'C', age: 30 }));
    records.set('4', createRecord({ name: 'D', age: 40 }));
    records.set('5', createRecord({ name: 'E', age: 50 }));
  });

  test('should filter records', () => {
    const query: Query = { where: { age: { $gt: 25 } } };
    const results = executeQuery(records, query);
    expect(results.length).toBe(3);
    expect(results.map(r => r.value.name).sort()).toEqual(['C', 'D', 'E']);
  });

  test('should sort records ascending', () => {
    const query: Query = { sort: { age: 'asc' } };
    const results = executeQuery(records, query);
    expect(results.map(r => r.value.name)).toEqual(['A', 'B', 'C', 'D', 'E']);
  });

  test('should sort records descending', () => {
    const query: Query = { sort: { age: 'desc' } };
    const results = executeQuery(records, query);
    expect(results.map(r => r.value.name)).toEqual(['E', 'D', 'C', 'B', 'A']);
  });

  test('should limit results', () => {
    const query: Query = { sort: { age: 'asc' }, limit: 2 };
    const results = executeQuery(records, query);
    expect(results.length).toBe(2);
    expect(results.map(r => r.value.name)).toEqual(['A', 'B']);
  });

  test('should apply cursor-based pagination (Phase 14.1)', () => {
    const { executeQueryWithCursor } = require('../Matcher');

    // First page
    const query1: Query = { sort: { age: 'asc' }, limit: 2 };
    const page1 = executeQueryWithCursor(records, query1);
    expect(page1.results.length).toBe(2);
    expect(page1.results.map((r: any) => r.value.name)).toEqual(['A', 'B']);
    expect(page1.hasMore).toBe(true);
    expect(page1.nextCursor).toBeDefined();

    // Second page using cursor
    const query2: Query = { sort: { age: 'asc' }, limit: 2, cursor: page1.nextCursor };
    const page2 = executeQueryWithCursor(records, query2);
    expect(page2.results.length).toBe(2);
    expect(page2.results.map((r: any) => r.value.name)).toEqual(['C', 'D']);
  });

  test('should combine filter, sort, limit with cursor', () => {
    const { executeQueryWithCursor } = require('../Matcher');

    // Filter > 15 (B, C, D, E) -> Sort Desc (E, D, C, B) -> Limit 2 (E, D)
    const query: Query = {
      where: { age: { $gt: 15 } },
      sort: { age: 'desc' },
      limit: 2
    };
    const result = executeQueryWithCursor(records, query);
    expect(result.results.length).toBe(2);
    expect(result.results[0].value.name).toBe('E');
    expect(result.results[1].value.name).toBe('D');
    expect(result.hasMore).toBe(true);
  });
});
