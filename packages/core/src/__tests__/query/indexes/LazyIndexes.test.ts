/**
 * Tests for Lazy Indexes
 *
 * Tests lazy index building functionality:
 * - LazyHashIndex
 * - LazyNavigableIndex
 * - LazyInvertedIndex
 * - Integration with IndexedLWWMap
 */

import { HLC } from '../../../HLC';
import { IndexedLWWMap } from '../../../IndexedLWWMap';
import { simpleAttribute } from '../../../query/Attribute';
import {
  LazyHashIndex,
  LazyNavigableIndex,
  LazyInvertedIndex,
  isLazyIndex,
} from '../../../query/indexes/lazy';
import type { IndexBuildProgressCallback } from '../../../query/indexes/lazy';

interface TestProduct {
  id: string;
  name: string;
  category: string;
  price: number;
  description: string;
}

describe('LazyHashIndex', () => {
  const categoryAttr = simpleAttribute<TestProduct, string>(
    'category',
    (p) => p.category
  );

  it('should buffer records before first query', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);

    expect(index.isBuilt).toBe(false);
    expect(index.pendingCount).toBe(0);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'B', price: 20, description: 'Test' });

    expect(index.isBuilt).toBe(false);
    expect(index.pendingCount).toBe(2);
    expect(index.getInnerIndex()).toBeNull();
  });

  it('should materialize on first query', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'A', price: 20, description: 'Test' });
    index.add('3', { id: '3', name: 'Product 3', category: 'B', price: 30, description: 'Test' });

    expect(index.isBuilt).toBe(false);

    const result = index.retrieve({ type: 'equal', value: 'A' });
    const keys = result.toArray();

    expect(index.isBuilt).toBe(true);
    expect(index.pendingCount).toBe(0);
    expect(keys).toHaveLength(2);
    expect(keys).toContain('1');
    expect(keys).toContain('2');
    expect(index.getInnerIndex()).not.toBeNull();
  });

  it('should call progress callback during materialization', () => {
    const progressCalls: Array<{ attr: string; progress: number; processed: number; total: number }> = [];
    const callback: IndexBuildProgressCallback = (attr, progress, processed, total) => {
      progressCalls.push({ attr, progress, processed, total });
    };

    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr, {
      onProgress: callback,
      progressBatchSize: 2,
    });

    // Add 5 records
    for (let i = 1; i <= 5; i++) {
      index.add(`${i}`, { id: `${i}`, name: `Product ${i}`, category: 'A', price: i * 10, description: 'Test' });
    }

    index.materialize();

    expect(progressCalls.length).toBeGreaterThan(0);
    expect(progressCalls[progressCalls.length - 1].progress).toBe(100);
    expect(progressCalls[progressCalls.length - 1].total).toBe(5);
  });

  it('should handle updates to pending records', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.update('1',
      { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' },
      { id: '1', name: 'Product 1', category: 'B', price: 10, description: 'Test' }
    );

    const result = index.retrieve({ type: 'equal', value: 'B' });
    const keys = result.toArray();

    expect(keys).toHaveLength(1);
    expect(keys).toContain('1');
  });

  it('should handle removal from pending records', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'A', price: 20, description: 'Test' });
    index.remove('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });

    expect(index.pendingCount).toBe(1);

    const result = index.retrieve({ type: 'equal', value: 'A' });
    expect(result.toArray()).toHaveLength(1);
    expect(result.toArray()).toContain('2');
  });

  it('should add to inner index after materialization', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.materialize();

    // Add after materialization
    index.add('2', { id: '2', name: 'Product 2', category: 'A', price: 20, description: 'Test' });

    const result = index.retrieve({ type: 'equal', value: 'A' });
    expect(result.toArray()).toHaveLength(2);
  });

  it('should be identified as lazy index', () => {
    const index = new LazyHashIndex<string, TestProduct, string>(categoryAttr);
    expect(isLazyIndex(index)).toBe(true);
  });
});

describe('LazyNavigableIndex', () => {
  const priceAttr = simpleAttribute<TestProduct, number>('price', (p) => p.price);

  it('should buffer records before first query', () => {
    const index = new LazyNavigableIndex<string, TestProduct, number>(priceAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'B', price: 20, description: 'Test' });

    expect(index.isBuilt).toBe(false);
    expect(index.pendingCount).toBe(2);
  });

  it('should materialize and support range queries', () => {
    const index = new LazyNavigableIndex<string, TestProduct, number>(priceAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'B', price: 25, description: 'Test' });
    index.add('3', { id: '3', name: 'Product 3', category: 'C', price: 50, description: 'Test' });

    // Query should trigger materialization
    const result = index.retrieve({ type: 'gte', value: 20 });
    const keys = result.toArray();

    expect(index.isBuilt).toBe(true);
    expect(keys).toHaveLength(2);
    expect(keys).toContain('2');
    expect(keys).toContain('3');
  });

  it('should support between queries', () => {
    const index = new LazyNavigableIndex<string, TestProduct, number>(priceAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'B', price: 25, description: 'Test' });
    index.add('3', { id: '3', name: 'Product 3', category: 'C', price: 50, description: 'Test' });

    const result = index.retrieve({
      type: 'between',
      from: 15,
      to: 40,
      fromInclusive: true,
      toInclusive: true,
    });

    expect(result.toArray()).toEqual(['2']);
  });

  it('should support getMinValue and getMaxValue', () => {
    const index = new LazyNavigableIndex<string, TestProduct, number>(priceAttr);

    index.add('1', { id: '1', name: 'Product 1', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Product 2', category: 'B', price: 50, description: 'Test' });

    // These should trigger materialization
    expect(index.getMinValue()).toBe(10);
    expect(index.getMaxValue()).toBe(50);
    expect(index.isBuilt).toBe(true);
  });
});

describe('LazyInvertedIndex', () => {
  const nameAttr = simpleAttribute<TestProduct, string>('name', (p) => p.name);

  it('should buffer records before first query', () => {
    const index = new LazyInvertedIndex<string, TestProduct, string>(nameAttr);

    index.add('1', { id: '1', name: 'Wireless Mouse', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Wireless Keyboard', category: 'B', price: 20, description: 'Test' });

    expect(index.isBuilt).toBe(false);
    expect(index.pendingCount).toBe(2);
  });

  it('should materialize and support contains queries', () => {
    const index = new LazyInvertedIndex<string, TestProduct, string>(nameAttr);

    index.add('1', { id: '1', name: 'Wireless Mouse', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Wireless Keyboard', category: 'B', price: 20, description: 'Test' });
    index.add('3', { id: '3', name: 'USB Cable', category: 'C', price: 5, description: 'Test' });

    // Query should trigger materialization
    const result = index.retrieve({ type: 'contains', value: 'wireless' as string });
    const keys = result.toArray();

    expect(index.isBuilt).toBe(true);
    expect(keys).toHaveLength(2);
    expect(keys).toContain('1');
    expect(keys).toContain('2');
  });

  it('should support hasToken and getTokenDocumentCount', () => {
    const index = new LazyInvertedIndex<string, TestProduct, string>(nameAttr);

    index.add('1', { id: '1', name: 'Wireless Mouse', category: 'A', price: 10, description: 'Test' });
    index.add('2', { id: '2', name: 'Wireless Keyboard', category: 'B', price: 20, description: 'Test' });

    // These should trigger materialization
    expect(index.hasToken('wireless')).toBe(true);
    expect(index.hasToken('nonexistent')).toBe(false);
    expect(index.getTokenDocumentCount('wireless')).toBe(2);
    expect(index.isBuilt).toBe(true);
  });
});

describe('IndexedLWWMap with Lazy Indexes', () => {
  const createProducts = (count: number): TestProduct[] => {
    return Array.from({ length: count }, (_, i) => ({
      id: `${i + 1}`,
      name: `Product ${i + 1}`,
      category: i % 3 === 0 ? 'A' : i % 3 === 1 ? 'B' : 'C',
      price: (i + 1) * 10,
      description: `Description for product ${i + 1}`,
    }));
  };

  it('should create lazy indexes when enabled', () => {
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc, {
      lazyIndexBuilding: true,
    });

    const categoryAttr = simpleAttribute<TestProduct, string>('category', (p) => p.category);
    const index = map.addHashIndex(categoryAttr);

    expect(map.isLazyIndexingEnabled()).toBe(true);
    expect('isLazy' in index).toBe(true);
  });

  it('should create regular indexes when lazy is not enabled', () => {
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc);

    const categoryAttr = simpleAttribute<TestProduct, string>('category', (p) => p.category);
    const index = map.addHashIndex(categoryAttr);

    expect(map.isLazyIndexingEnabled()).toBe(false);
    expect('isLazy' in index).toBe(false);
  });

  it('should defer index building until first query', () => {
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc, {
      lazyIndexBuilding: true,
    });

    const categoryAttr = simpleAttribute<TestProduct, string>('category', (p) => p.category);
    map.addHashIndex(categoryAttr);

    // Add products
    const products = createProducts(100);
    for (const product of products) {
      map.set(product.id, product);
    }

    // Index should not be built yet
    expect(map.hasUnbuiltIndexes()).toBe(true);
    expect(map.getPendingIndexCount()).toBe(100);

    // Query should trigger materialization
    const result = map.query({ type: 'eq', attribute: 'category', value: 'A' });

    expect(map.hasUnbuiltIndexes()).toBe(false);
    expect(map.getPendingIndexCount()).toBe(0);
    expect(result.size()).toBeGreaterThan(0);
  });

  it('should call onIndexBuilding callback', () => {
    const progressCalls: Array<{ attr: string; progress: number }> = [];
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc, {
      lazyIndexBuilding: true,
      onIndexBuilding: (attr, progress) => {
        progressCalls.push({ attr, progress });
      },
    });

    const categoryAttr = simpleAttribute<TestProduct, string>('category', (p) => p.category);
    map.addHashIndex(categoryAttr);

    const products = createProducts(10);
    for (const product of products) {
      map.set(product.id, product);
    }

    // Trigger query
    map.query({ type: 'eq', attribute: 'category', value: 'A' });

    expect(progressCalls.length).toBeGreaterThan(0);
    expect(progressCalls[progressCalls.length - 1].progress).toBe(100);
  });

  it('should support materializeAllIndexes()', () => {
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc, {
      lazyIndexBuilding: true,
    });

    const categoryAttr = simpleAttribute<TestProduct, string>('category', (p) => p.category);
    const priceAttr = simpleAttribute<TestProduct, number>('price', (p) => p.price);

    map.addHashIndex(categoryAttr);
    map.addNavigableIndex(priceAttr);

    const products = createProducts(50);
    for (const product of products) {
      map.set(product.id, product);
    }

    expect(map.hasUnbuiltIndexes()).toBe(true);

    // Force materialization
    map.materializeAllIndexes();

    expect(map.hasUnbuiltIndexes()).toBe(false);
    expect(map.getPendingIndexCount()).toBe(0);
  });

  it('should support lazy inverted index for text search', () => {
    const hlc = new HLC('test-node');
    const map = new IndexedLWWMap<string, TestProduct>(hlc, {
      lazyIndexBuilding: true,
    });

    const nameAttr = simpleAttribute<TestProduct, string>('name', (p) => p.name);
    map.addInvertedIndex(nameAttr);

    map.set('1', { id: '1', name: 'Wireless Mouse', category: 'A', price: 10, description: 'Test' });
    map.set('2', { id: '2', name: 'Wireless Keyboard', category: 'B', price: 20, description: 'Test' });
    map.set('3', { id: '3', name: 'USB Cable', category: 'C', price: 5, description: 'Test' });

    expect(map.hasUnbuiltIndexes()).toBe(true);

    // Query should trigger materialization
    const result = map.query({ type: 'contains', attribute: 'name', value: 'wireless' });
    const keys = result.toArray();

    expect(map.hasUnbuiltIndexes()).toBe(false);
    expect(keys).toHaveLength(2);
  });
});
