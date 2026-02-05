/**
 * Point Lookup Optimization Tests
 *
 * Tests for O(1) direct key access optimization in QueryOptimizer and QueryExecutor.
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { QueryExecutor } from '../../query/QueryExecutor';
import { IndexRegistry } from '../../query/IndexRegistry';
import { StandingQueryRegistry } from '../../query/StandingQueryRegistry';
import type { Query, PointLookupStep, MultiPointLookupStep } from '../../query/QueryTypes';
import { HashIndex } from '../../query/indexes/HashIndex';
import { simpleAttribute } from '../../query/Attribute';

describe('Point Lookup Optimization', () => {
  let indexRegistry: IndexRegistry<string, Record<string, unknown>>;
  let standingQueryRegistry: StandingQueryRegistry<string, Record<string, unknown>>;
  let optimizer: QueryOptimizer<string, Record<string, unknown>>;
  let executor: QueryExecutor<string, Record<string, unknown>>;
  let data: Map<string, Record<string, unknown>>;

  beforeEach(() => {
    indexRegistry = new IndexRegistry<string, Record<string, unknown>>();
    standingQueryRegistry = new StandingQueryRegistry<string, Record<string, unknown>>();
    optimizer = new QueryOptimizer({
      indexRegistry,
      standingQueryRegistry,
    });
    executor = new QueryExecutor(optimizer);

    // Create test data
    data = new Map([
      ['user-1', { id: 'user-1', name: 'Alice', age: 30 }],
      ['user-2', { id: 'user-2', name: 'Bob', age: 25 }],
      ['user-3', { id: 'user-3', name: 'Charlie', age: 35 }],
    ]);
  });

  describe('Point Lookup on "id" field', () => {
    it('should return correct result with cost 1', () => {
      const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const plan = optimizer.optimize(query);

      // Check plan structure
      expect(plan.root.type).toBe('point-lookup');
      expect(plan.estimatedCost).toBe(1);
      expect(plan.usesIndexes).toBe(true);

      const pointLookupStep = plan.root as PointLookupStep;
      expect(pointLookupStep.key).toBe('user-1');
      expect(pointLookupStep.cost).toBe(1);

      // Execute and verify result
      const results = executor.execute(query, data);
      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('user-1');
      expect(results[0].value).toEqual({ id: 'user-1', name: 'Alice', age: 30 });
    });
  });

  describe('Point Lookup on "_key" field', () => {
    it('should return correct result', () => {
      const query: Query = { type: 'eq', attribute: '_key', value: 'user-2' };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('point-lookup');
      expect(plan.estimatedCost).toBe(1);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('user-2');
      expect(results[0].value.name).toBe('Bob');
    });
  });

  describe('Point Lookup on "key" field', () => {
    it('should return correct result', () => {
      const query: Query = { type: 'eq', attribute: 'key', value: 'user-3' };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('point-lookup');
      expect(plan.estimatedCost).toBe(1);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('user-3');
      expect(results[0].value.name).toBe('Charlie');
    });
  });

  describe('Point Lookup for non-existent key', () => {
    it('should return empty result', () => {
      const query: Query = { type: 'eq', attribute: 'id', value: 'user-999' };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('point-lookup');
      expect(plan.estimatedCost).toBe(1);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(0);
    });
  });

  describe('Multi-point lookup', () => {
    it('should return all existing keys', () => {
      const query: Query = {
        type: 'in',
        attribute: '_key',
        values: ['user-1', 'user-2', 'user-3'],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('multi-point-lookup');
      expect(plan.estimatedCost).toBe(3);

      const multiPointStep = plan.root as MultiPointLookupStep;
      expect(multiPointStep.keys).toEqual(['user-1', 'user-2', 'user-3']);
      expect(multiPointStep.cost).toBe(3);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(3);

      const keys = results.map((r) => r.key).sort();
      expect(keys).toEqual(['user-1', 'user-2', 'user-3']);
    });

    it('should return only existing keys when some are missing', () => {
      const query: Query = {
        type: 'in',
        attribute: 'id',
        values: ['user-1', 'user-999', 'user-2', 'user-888'],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('multi-point-lookup');
      expect(plan.estimatedCost).toBe(4);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(2);

      const keys = results.map((r) => r.key).sort();
      expect(keys).toEqual(['user-1', 'user-2']);
    });

    it('should return empty result with cost 0 for empty values array', () => {
      const query: Query = {
        type: 'in',
        attribute: 'key',
        values: [],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('multi-point-lookup');
      expect(plan.estimatedCost).toBe(0);

      const multiPointStep = plan.root as MultiPointLookupStep;
      expect(multiPointStep.keys).toEqual([]);
      expect(multiPointStep.cost).toBe(0);

      const results = executor.execute(query, data);
      expect(results).toHaveLength(0);
    });
  });

  describe('Non-key attribute queries', () => {
    it('should use normal optimization path for equality on non-key field', () => {
      const query: Query = { type: 'eq', attribute: 'name', value: 'Alice' };
      const plan = optimizer.optimize(query);

      // Should NOT be point-lookup
      expect(plan.root.type).not.toBe('point-lookup');
      // Should be full-scan (no index on 'name')
      expect(plan.root.type).toBe('full-scan');

      const results = executor.execute(query, data);
      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Alice');
    });
  });

  describe('Point lookup prioritization', () => {
    it('should prioritize point lookup over StandingQueryIndex', () => {
      // Register a standing query index for the same query
      const query: Query = { type: 'eq', attribute: 'id', value: 'user-1' };
      const standingIndex = new HashIndex({
        attribute: simpleAttribute('id'),
        keyExtractor: (v) => v.id as string,
      });

      standingQueryRegistry.register(query, standingIndex);

      // Despite having a standing query, point lookup should be used (cost 1 < 10)
      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('point-lookup');
      expect(plan.estimatedCost).toBe(1);
    });
  });

  describe('All existing tests compatibility', () => {
    it('should not affect range queries', () => {
      const query: Query = { type: 'gt', attribute: 'age', value: 28 };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).not.toBe('point-lookup');
      expect(plan.root.type).toBe('full-scan');
    });

    it('should not affect AND queries', () => {
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'name', value: 'Alice' },
          { type: 'gt', attribute: 'age', value: 25 },
        ],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).not.toBe('point-lookup');
    });

    it('should not affect OR queries', () => {
      const query: Query = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'name', value: 'Alice' },
          { type: 'eq', attribute: 'name', value: 'Bob' },
        ],
      };
      const plan = optimizer.optimize(query);

      expect(plan.root.type).not.toBe('point-lookup');
    });
  });
});
