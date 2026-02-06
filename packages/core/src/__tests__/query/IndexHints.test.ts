/**
 * Index Hints Tests
 *
 * Tests for useIndex, forceIndexScan, and disableOptimization
 * options in QueryOptimizer.optimizeWithOptions().
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import { HashIndex } from '../../query/indexes/HashIndex';
import { NavigableIndex } from '../../query/indexes/NavigableIndex';
import { simpleAttribute } from '../../query/Attribute';
import type { SimpleQueryNode, LogicalQueryNode } from '../../query/QueryTypes';

interface TestRecord {
  id: string;
  name: string;
  age: number;
  status: string;
  category: string;
}

describe('Index Hints', () => {
  let registry: IndexRegistry<string, TestRecord>;
  let optimizer: QueryOptimizer<string, TestRecord>;

  beforeEach(() => {
    registry = new IndexRegistry<string, TestRecord>();
    optimizer = new QueryOptimizer({ indexRegistry: registry });
  });

  describe('useIndex', () => {
    it('uses specified attribute index when available', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, { useIndex: 'status' });

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.index).toBe(statusIndex);
      }
    });

    it('throws when no index exists for specified attribute', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(() => optimizer.optimizeWithOptions(query, { useIndex: 'nonexistent' })).toThrow(
        'Index hint: no index found for attribute "nonexistent"'
      );
    });

    it('picks lowest-cost index when multiple indexes exist for attribute', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const hashIndex = new HashIndex<string, TestRecord, string>(statusAttr); // cost 30
      const navIndex = new NavigableIndex<string, TestRecord, string>(statusAttr); // cost 40
      registry.addIndex(hashIndex);
      registry.addIndex(navIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, { useIndex: 'status' });

      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        // Hash (30) is cheaper than Navigable (40)
        expect(plan.root.index).toBe(hashIndex);
      }
      expect(plan.estimatedCost).toBe(30);
    });

    it('applies sort/limit options alongside useIndex hint', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const ageAttr = simpleAttribute<TestRecord, number>('age', (r) => r.age);
      const ageIndex = new NavigableIndex<string, TestRecord, number>(ageAttr);
      registry.addIndex(ageIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, {
        useIndex: 'status',
        sort: { age: 'asc' },
        limit: 10,
      });

      expect(plan.usesIndexes).toBe(true);
      expect(plan.hint).toBe('status');
      expect(plan.sort).toEqual({ field: 'age', direction: 'asc' });
      expect(plan.limit).toBe(10);
      // age has a NavigableIndex so indexedSort should be true
      expect(plan.indexedSort).toBe(true);
    });

    it('sets hint field on returned QueryPlan', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, { useIndex: 'status' });

      expect(plan.hint).toBe('status');
    });

    it('extracts matching predicate from AND query for hinted attribute', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const child1: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const child2: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const query: LogicalQueryNode = {
        type: 'and',
        children: [child1, child2],
      };

      const plan = optimizer.optimizeWithOptions(query, { useIndex: 'status' });

      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        // Should extract the status predicate, not fall back to 'has'
        expect(plan.root.query.type).toBe('equal');
        expect(plan.root.query.value).toBe('active');
      }
    });

    it('falls back to "has" query when no matching predicate found', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      // Query references 'name', not 'status', so no matching predicate for the hint
      const query: SimpleQueryNode = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimizeWithOptions(query, { useIndex: 'status' });

      expect(plan.root.type).toBe('index-scan');
      if (plan.root.type === 'index-scan') {
        expect(plan.root.query.type).toBe('has');
      }
    });
  });

  describe('forceIndexScan', () => {
    it('passes when plan uses indexes', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // Should not throw because an index exists
      expect(() =>
        optimizer.optimizeWithOptions(query, { forceIndexScan: true })
      ).not.toThrow();

      const plan = optimizer.optimizeWithOptions(query, { forceIndexScan: true });
      expect(plan.usesIndexes).toBe(true);
    });

    it('throws when plan would be full-scan', () => {
      // No indexes registered
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      expect(() =>
        optimizer.optimizeWithOptions(query, { forceIndexScan: true })
      ).toThrow('No suitable index found and forceIndexScan is enabled');
    });
  });

  describe('disableOptimization', () => {
    it('returns full-scan plan regardless of available indexes', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, { disableOptimization: true });

      expect(plan.root.type).toBe('full-scan');
      expect(plan.usesIndexes).toBe(false);
      expect(plan.estimatedCost).toBe(Number.MAX_SAFE_INTEGER);
    });

    it('does not apply sort/limit/cursor', () => {
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, {
        disableOptimization: true,
        sort: { status: 'asc' },
        limit: 10,
        cursor: 'abc',
      });

      expect(plan.root.type).toBe('full-scan');
      expect(plan.sort).toBeUndefined();
      expect(plan.limit).toBeUndefined();
      expect(plan.cursor).toBeUndefined();
    });
  });

  describe('option combinations', () => {
    it('disableOptimization takes precedence over useIndex', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, {
        disableOptimization: true,
        useIndex: 'status',
      });

      // disableOptimization wins: full-scan, no index
      expect(plan.root.type).toBe('full-scan');
      expect(plan.usesIndexes).toBe(false);
      expect(plan.hint).toBeUndefined();
    });

    it('disableOptimization takes precedence over forceIndexScan', () => {
      // No indexes registered, but forceIndexScan + disableOptimization
      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };

      // forceIndexScan would throw, but disableOptimization takes priority
      expect(() =>
        optimizer.optimizeWithOptions(query, {
          disableOptimization: true,
          forceIndexScan: true,
        })
      ).not.toThrow();

      const plan = optimizer.optimizeWithOptions(query, {
        disableOptimization: true,
        forceIndexScan: true,
      });
      expect(plan.root.type).toBe('full-scan');
      expect(plan.usesIndexes).toBe(false);
    });

    it('useIndex and forceIndexScan together: useIndex takes priority (plan always has index)', () => {
      const statusAttr = simpleAttribute<TestRecord, string>('status', (r) => r.status);
      const statusIndex = new HashIndex<string, TestRecord, string>(statusAttr);
      registry.addIndex(statusIndex);

      const query: SimpleQueryNode = { type: 'eq', attribute: 'status', value: 'active' };
      const plan = optimizer.optimizeWithOptions(query, {
        useIndex: 'status',
        forceIndexScan: true,
      });

      // useIndex produces an index-scan plan, forceIndexScan is redundant
      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
      expect(plan.hint).toBe('status');
    });
  });
});
