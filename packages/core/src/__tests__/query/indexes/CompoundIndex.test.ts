/**
 * Tests for CompoundIndex (Phase 9.03)
 *
 * Tests multi-attribute indexing functionality:
 * - Composite key generation
 * - O(1) lookup for compound queries
 * - Query matching logic
 * - Index maintenance (add/remove/update)
 */

import { CompoundIndex, isCompoundIndex } from '../../../query/indexes/CompoundIndex';
import { SimpleAttribute } from '../../../query/Attribute';

interface TestProduct {
  id: string;
  status: 'active' | 'inactive' | 'pending';
  category: string;
  region: string;
  price: number;
}

const createProduct = (overrides: Partial<TestProduct> = {}): TestProduct => ({
  id: 'prod-1',
  status: 'active',
  category: 'electronics',
  region: 'US',
  price: 100,
  ...overrides,
});

describe('CompoundIndex', () => {
  const statusAttr = new SimpleAttribute<TestProduct, string>('status', (p) => p.status);
  const categoryAttr = new SimpleAttribute<TestProduct, string>('category', (p) => p.category);
  const regionAttr = new SimpleAttribute<TestProduct, string>('region', (p) => p.region);

  describe('constructor', () => {
    it('should require at least 2 attributes', () => {
      expect(() => new CompoundIndex([statusAttr])).toThrow(
        'CompoundIndex requires at least 2 attributes'
      );
    });

    it('should create index with 2 attributes', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.type).toBe('compound');
      expect(index.attributes.length).toBe(2);
    });

    it('should create index with 3+ attributes', () => {
      const index = new CompoundIndex<string, TestProduct>([
        statusAttr,
        categoryAttr,
        regionAttr,
      ]);
      expect(index.attributes.length).toBe(3);
    });

    it('should use custom separator', () => {
      const index = new CompoundIndex<string, TestProduct>(
        [statusAttr, categoryAttr],
        { separator: '::' }
      );
      expect(index.compoundName).toBe('status+category');
    });
  });

  describe('attribute accessors', () => {
    it('should return first attribute via attribute getter', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.attribute.name).toBe('status');
    });

    it('should return all attributes via attributes getter', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      const attrs = index.attributes;
      expect(attrs.length).toBe(2);
      expect(attrs[0].name).toBe('status');
      expect(attrs[1].name).toBe('category');
    });

    it('should return compound name', () => {
      const index = new CompoundIndex<string, TestProduct>([
        statusAttr,
        categoryAttr,
        regionAttr,
      ]);
      expect(index.compoundName).toBe('status+category+region');
    });
  });

  describe('add and retrieve', () => {
    it('should add and retrieve single record', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      const product = createProduct();

      index.add('prod-1', product);

      const result = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual(['prod-1']);
    });

    it('should retrieve multiple records with same composite key', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct({ id: 'prod-1' }));
      index.add('prod-2', createProduct({ id: 'prod-2' })); // Same status + category

      const result = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray().sort()).toEqual(['prod-1', 'prod-2']);
    });

    it('should distinguish different composite keys', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct({ status: 'active', category: 'electronics' }));
      index.add('prod-2', createProduct({ status: 'active', category: 'clothing' }));
      index.add('prod-3', createProduct({ status: 'inactive', category: 'electronics' }));

      const result1 = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result1.toArray()).toEqual(['prod-1']);

      const result2 = index.retrieve({ type: 'compound', values: ['active', 'clothing'] });
      expect(result2.toArray()).toEqual(['prod-2']);

      const result3 = index.retrieve({ type: 'compound', values: ['inactive', 'electronics'] });
      expect(result3.toArray()).toEqual(['prod-3']);
    });

    it('should return empty result for non-existent combination', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct({ status: 'active', category: 'electronics' }));

      const result = index.retrieve({ type: 'compound', values: ['pending', 'furniture'] });
      expect(result.toArray()).toEqual([]);
    });
  });

  describe('retrieveByValues convenience method', () => {
    it('should work as shorthand for retrieve', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct());

      const result = index.retrieveByValues('active', 'electronics');
      expect(result.toArray()).toEqual(['prod-1']);
    });
  });

  describe('remove', () => {
    it('should remove record from index', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      const product = createProduct();

      index.add('prod-1', product);
      index.remove('prod-1', product);

      const result = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual([]);
    });

    it('should keep other records when removing one', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      const prod1 = createProduct({ id: 'prod-1' });
      const prod2 = createProduct({ id: 'prod-2' });

      index.add('prod-1', prod1);
      index.add('prod-2', prod2);
      index.remove('prod-1', prod1);

      const result = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual(['prod-2']);
    });
  });

  describe('update', () => {
    it('should update when composite key changes', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      const oldProduct = createProduct({ status: 'active' });
      const newProduct = createProduct({ status: 'inactive' });

      index.add('prod-1', oldProduct);
      index.update('prod-1', oldProduct, newProduct);

      // Should not be in old location
      const oldResult = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(oldResult.toArray()).toEqual([]);

      // Should be in new location
      const newResult = index.retrieve({ type: 'compound', values: ['inactive', 'electronics'] });
      expect(newResult.toArray()).toEqual(['prod-1']);
    });

    it('should optimize when composite key unchanged', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      const oldProduct = createProduct({ price: 100 });
      const newProduct = createProduct({ price: 200 }); // Same status + category

      index.add('prod-1', oldProduct);
      index.update('prod-1', oldProduct, newProduct);

      // Should still be findable
      const result = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      expect(result.toArray()).toEqual(['prod-1']);
    });
  });

  describe('clear', () => {
    it('should remove all entries', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct({ id: 'prod-1' }));
      index.add('prod-2', createProduct({ id: 'prod-2', category: 'clothing' }));

      index.clear();

      const result1 = index.retrieve({ type: 'compound', values: ['active', 'electronics'] });
      const result2 = index.retrieve({ type: 'compound', values: ['active', 'clothing'] });

      expect(result1.toArray()).toEqual([]);
      expect(result2.toArray()).toEqual([]);
    });
  });

  describe('supportsQuery', () => {
    it('should support compound query type', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.supportsQuery('compound')).toBe(true);
    });

    it('should not support other query types', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.supportsQuery('eq')).toBe(false);
      expect(index.supportsQuery('range')).toBe(false);
    });
  });

  describe('retrieve error handling', () => {
    it('should throw on non-compound query type', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      expect(() =>
        index.retrieve({ type: 'eq', value: 'active' } as any)
      ).toThrow("CompoundIndex only supports 'compound' query type");
    });

    it('should throw when values count mismatches attributes', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      expect(() =>
        index.retrieve({ type: 'compound', values: ['active'] } as any)
      ).toThrow('CompoundIndex requires 2 values, got 1');

      expect(() =>
        index.retrieve({ type: 'compound', values: ['a', 'b', 'c'] } as any)
      ).toThrow('CompoundIndex requires 2 values, got 3');
    });
  });

  describe('canAnswerQuery', () => {
    it('should return true for matching attribute names', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.canAnswerQuery(['status', 'category'])).toBe(true);
    });

    it('should return false for wrong order', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.canAnswerQuery(['category', 'status'])).toBe(false);
    });

    it('should return false for partial match', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.canAnswerQuery(['status'])).toBe(false);
    });

    it('should return false for different attributes', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(index.canAnswerQuery(['status', 'region'])).toBe(false);
    });
  });

  describe('getStats and getExtendedStats', () => {
    it('should return basic stats', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);

      index.add('prod-1', createProduct({ status: 'active', category: 'electronics' }));
      index.add('prod-2', createProduct({ status: 'active', category: 'electronics' }));
      index.add('prod-3', createProduct({ status: 'inactive', category: 'clothing' }));

      const stats = index.getStats();
      expect(stats.distinctValues).toBe(2); // 2 unique composite keys
      expect(stats.totalEntries).toBe(3);
    });

    it('should return extended stats', () => {
      const index = new CompoundIndex<string, TestProduct>([
        statusAttr,
        categoryAttr,
        regionAttr,
      ]);

      index.add('prod-1', createProduct());

      const stats = index.getExtendedStats();
      expect(stats.attributeCount).toBe(3);
      expect(stats.attributeNames).toEqual(['status', 'category', 'region']);
      expect(stats.compositeKeyCount).toBe(1);
    });
  });

  describe('getRetrievalCost', () => {
    it('should return low retrieval cost (O(1) lookup)', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      // Compound indexes should have low cost since they're O(1) lookups
      expect(index.getRetrievalCost()).toBeLessThan(100);
    });
  });

  describe('handling undefined values', () => {
    it('should skip records with undefined attribute values', () => {
      const optionalAttr = new SimpleAttribute<TestProduct, string | undefined>(
        'optional',
        () => undefined
      );
      const index = new CompoundIndex<string, TestProduct>([statusAttr, optionalAttr]);

      // This should not throw and should not add to index
      index.add('prod-1', createProduct());

      const stats = index.getStats();
      expect(stats.totalEntries).toBe(0);
    });
  });

  describe('special value encoding', () => {
    it('should handle null values', () => {
      const nullableAttr = new SimpleAttribute<TestProduct, string | null>(
        'nullable',
        () => null
      );
      const index = new CompoundIndex<string, TestProduct>([statusAttr, nullableAttr]);

      index.add('prod-1', createProduct());

      const result = index.retrieve({ type: 'compound', values: ['active', null] });
      expect(result.toArray()).toEqual(['prod-1']);
    });

    it('should handle values containing separator', () => {
      const pipeAttr = new SimpleAttribute<TestProduct, string>('pipe', () => 'value|with|pipes');
      const index = new CompoundIndex<string, TestProduct>([statusAttr, pipeAttr]);

      index.add('prod-1', createProduct());

      const result = index.retrieve({ type: 'compound', values: ['active', 'value|with|pipes'] });
      expect(result.toArray()).toEqual(['prod-1']);
    });
  });

  describe('isCompoundIndex helper', () => {
    it('should return true for CompoundIndex', () => {
      const index = new CompoundIndex<string, TestProduct>([statusAttr, categoryAttr]);
      expect(isCompoundIndex(index)).toBe(true);
    });

    it('should return false for other indexes', () => {
      const mockHashIndex = { type: 'hash' } as any;
      expect(isCompoundIndex(mockHashIndex)).toBe(false);
    });
  });
});
