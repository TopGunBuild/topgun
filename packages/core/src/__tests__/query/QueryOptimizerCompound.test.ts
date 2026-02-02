/**
 * Tests for QueryOptimizer Compound Index Usage
 *
 * Tests automatic detection and usage of CompoundIndex for AND queries:
 * - Automatic compound index selection
 * - Fallback to intersection when no compound index
 * - Mixed queries (eq + other types)
 */

import { QueryOptimizer } from '../../query/QueryOptimizer';
import { IndexRegistry } from '../../query/IndexRegistry';
import { HashIndex } from '../../query/indexes/HashIndex';
import { CompoundIndex } from '../../query/indexes/CompoundIndex';
import { SimpleAttribute } from '../../query/Attribute';
import type { Query, IndexScanStep, FilterStep, IntersectionStep } from '../../query/QueryTypes';

interface TestProduct {
  id: string;
  status: 'active' | 'inactive' | 'pending';
  category: string;
  region: string;
  price: number;
}

describe('QueryOptimizer Compound Index Usage', () => {
  const statusAttr = new SimpleAttribute<TestProduct, string>('status', (p) => p.status);
  const categoryAttr = new SimpleAttribute<TestProduct, string>('category', (p) => p.category);
  const regionAttr = new SimpleAttribute<TestProduct, string>('region', (p) => p.region);

  describe('automatic compound index detection', () => {
    it('should use compound index for AND of two eq queries', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
        ],
      };

      const plan = optimizer.optimize(query);

      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');

      const scanStep = plan.root as IndexScanStep;
      expect(scanStep.index.type).toBe('compound');
      expect(scanStep.query.type).toBe('compound');
      expect(scanStep.query.values).toEqual(['active', 'electronics']);
    });

    it('should use compound index for AND of three eq queries', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([
        statusAttr,
        categoryAttr,
        regionAttr,
      ]);
      registry.addIndex(compoundIndex);

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
          { type: 'eq', attribute: 'region', value: 'US' },
        ],
      };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('index-scan');
      const scanStep = plan.root as IndexScanStep;
      expect(scanStep.index.type).toBe('compound');
      expect(scanStep.query.values).toEqual(['active', 'electronics', 'US']);
    });

    it('should order values according to compound index attribute order', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      // Index order: status, category
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      // Query order: category first, then status
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'category', value: 'electronics' },
          { type: 'eq', attribute: 'status', value: 'active' },
        ],
      };

      const plan = optimizer.optimize(query);

      const scanStep = plan.root as IndexScanStep;
      // Values should be in index order (status, category), not query order
      expect(scanStep.query.values).toEqual(['active', 'electronics']);
    });
  });

  describe('fallback to intersection', () => {
    it('should use intersection when no compound index exists', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      // Only individual indexes, no compound
      registry.addIndex(new HashIndex(statusAttr));
      registry.addIndex(new HashIndex(categoryAttr));

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
        ],
      };

      const plan = optimizer.optimize(query);

      expect(plan.root.type).toBe('intersection');
      const intersectionStep = plan.root as IntersectionStep;
      expect(intersectionStep.steps.length).toBe(2);
    });

    it('should use intersection when compound index has wrong attributes', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      // Compound index on different attributes
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, regionAttr]);
      registry.addIndex(compoundIndex);
      registry.addIndex(new HashIndex(statusAttr));
      registry.addIndex(new HashIndex(categoryAttr));

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      // Query on status + category (not covered by compound index)
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
        ],
      };

      const plan = optimizer.optimize(query);

      // Should use intersection of individual indexes
      expect(plan.root.type).toBe('intersection');
    });
  });

  describe('mixed queries (eq + other types)', () => {
    it('should use compound index + filter for mixed AND query', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      // AND of two eq queries + a gt query
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
          { type: 'gt', attribute: 'price', value: 100 },
        ],
      };

      const plan = optimizer.optimize(query);

      // Should use compound index with filter
      expect(plan.root.type).toBe('filter');
      const filterStep = plan.root as FilterStep;
      expect(filterStep.source.type).toBe('index-scan');

      const scanStep = filterStep.source as IndexScanStep;
      expect(scanStep.index.type).toBe('compound');
      expect(scanStep.query.values).toEqual(['active', 'electronics']);

      // Filter predicate should be the gt query
      expect(filterStep.predicate).toEqual({ type: 'gt', attribute: 'price', value: 100 });
    });

    it('should fall back to intersection when only one eq query', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);
      registry.addIndex(new HashIndex(statusAttr));

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      // AND with one eq and one gt - can't use compound index
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'gt', attribute: 'price', value: 100 },
        ],
      };

      const plan = optimizer.optimize(query);

      // Should use single index + filter (not compound)
      expect(plan.root.type).toBe('filter');
      const filterStep = plan.root as FilterStep;
      expect(filterStep.source.type).toBe('index-scan');

      const scanStep = filterStep.source as IndexScanStep;
      expect(scanStep.index.type).toBe('hash'); // Uses hash index, not compound
    });
  });

  describe('IndexRegistry compound methods', () => {
    it('should register and find compound indexes', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      registry.addIndex(compoundIndex);

      expect(registry.hasCompoundIndex(['status', 'category'])).toBe(true);
      expect(registry.hasCompoundIndex(['category', 'status'])).toBe(true); // Order independent
      expect(registry.hasCompoundIndex(['status', 'region'])).toBe(false);
    });

    it('should count compound indexes in size', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      registry.addIndex(new HashIndex(statusAttr));
      registry.addIndex(new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]));

      expect(registry.size).toBe(2); // 1 hash + 1 compound
    });

    it('should return compound indexes in getCompoundIndexes', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compound1 = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      const compound2 = new CompoundIndex<string, TestProduct>([statusAttr, regionAttr]);

      registry.addIndex(compound1);
      registry.addIndex(compound2);

      const compoundIndexes = registry.getCompoundIndexes();
      expect(compoundIndexes.length).toBe(2);
    });

    it('should update compound indexes on record add/update/remove', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);

      const product: TestProduct = {
        id: 'prod-1',
        status: 'active',
        category: 'electronics',
        region: 'US',
        price: 100,
      };

      // Add
      registry.onRecordAdded('prod-1', product);
      let result = compoundIndex.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual(['prod-1']);

      // Update
      const updatedProduct = { ...product, status: 'inactive' as const };
      registry.onRecordUpdated('prod-1', product, updatedProduct);
      result = compoundIndex.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual([]);
      result = compoundIndex.retrieve({ type: 'compound', values: ['inactive', 'electronics'] });
      expect(result.toArray()).toEqual(['prod-1']);

      // Remove
      registry.onRecordRemoved('prod-1', updatedProduct);
      result = compoundIndex.retrieve({ type: 'compound', values: ['inactive', 'electronics'] });
      expect(result.toArray()).toEqual([]);
    });

    it('should include compound indexes in stats', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      registry.addIndex(new HashIndex(statusAttr));
      registry.addIndex(new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]));

      const stats = registry.getStats();
      expect(stats.totalIndexes).toBe(2);
      expect(stats.compoundIndexes).toBe(1);
    });
  });

  describe('cost estimation', () => {
    it('should prefer compound index over intersection (lower cost)', () => {
      const registry = new IndexRegistry<string, TestProduct>();
      const compoundIndex = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      registry.addIndex(compoundIndex);
      registry.addIndex(new HashIndex(statusAttr));
      registry.addIndex(new HashIndex(categoryAttr));

      const optimizer = new QueryOptimizer({ indexRegistry: registry });

      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'status', value: 'active' },
          { type: 'eq', attribute: 'category', value: 'electronics' },
        ],
      };

      const plan = optimizer.optimize(query);

      // Should use compound index (cost ~20) over intersection (cost ~30+30)
      expect(plan.root.type).toBe('index-scan');
      const scanStep = plan.root as IndexScanStep;
      expect(scanStep.index.type).toBe('compound');
      expect(plan.estimatedCost).toBeLessThan(60); // Less than intersection cost
    });
  });
});
