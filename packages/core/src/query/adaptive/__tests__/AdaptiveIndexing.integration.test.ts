/**
 * Integration Tests for Adaptive Indexing (Phase 8.02)
 *
 * Tests the complete adaptive indexing flow with IndexedLWWMap.
 */

import { HLC } from '../../../HLC';
import { IndexedLWWMap } from '../../../IndexedLWWMap';
import { simpleAttribute } from '../../Attribute';
import type { IndexedMapOptions } from '../types';

interface Product {
  id: string;
  name: string;
  category: string;
  price: number;
  inStock: boolean;
  description: string;
}

describe('Adaptive Indexing Integration', () => {
  let hlc: HLC;

  beforeEach(() => {
    hlc = new HLC('test-node');
  });

  describe('Index Advisor (Production Mode)', () => {
    it('tracks query patterns and provides suggestions', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          advisor: { enabled: true, minQueryCount: 5, minAverageCost: 0 },
        },
      });

      // Add data
      for (let i = 0; i < 100; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: i % 2 === 0,
          description: `Description for product ${i}`,
        });
      }

      // Execute queries without index
      for (let i = 0; i < 10; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      // Check statistics
      const stats = products.getQueryStatistics();
      expect(stats.length).toBeGreaterThan(0);

      const categoryStats = stats.find(s => s.attribute === 'category');
      expect(categoryStats).toBeDefined();
      expect(categoryStats?.queryCount).toBe(10);

      // Get suggestions (use options matching advisor config)
      const suggestions = products.getIndexSuggestions({ minQueryCount: 5, minAverageCost: 0 });
      expect(suggestions.length).toBeGreaterThan(0);

      const categorySuggestion = suggestions.find(s => s.attribute === 'category');
      expect(categorySuggestion).toBeDefined();
      expect(categorySuggestion?.indexType).toBe('hash');
    });

    it('suggests different index types based on query types', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          advisor: { enabled: true, minQueryCount: 5, minAverageCost: 0 },
        },
      });

      // Add data
      for (let i = 0; i < 100; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: i % 2 === 0,
          description: `Description ${i}`,
        });
      }

      // Execute equality queries on category
      for (let i = 0; i < 10; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      // Execute range queries on price
      for (let i = 0; i < 10; i++) {
        products.query({ type: 'gt', attribute: 'price', value: 500 });
      }

      const suggestions = products.getIndexSuggestions({ minQueryCount: 5, minAverageCost: 0 });

      const categorySuggestion = suggestions.find(s => s.attribute === 'category');
      const priceSuggestion = suggestions.find(s => s.attribute === 'price');

      expect(categorySuggestion?.indexType).toBe('hash');
      expect(priceSuggestion?.indexType).toBe('navigable');
    });

    it('excludes already indexed attributes from suggestions', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          advisor: { enabled: true, minQueryCount: 5, minAverageCost: 0 },
        },
      });

      // Add data
      for (let i = 0; i < 100; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: i % 2 === 0,
          description: `Description ${i}`,
        });
      }

      // Manually add index for category
      products.addHashIndex(simpleAttribute<Product, string>('category', p => p.category));

      // Execute queries
      for (let i = 0; i < 10; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      const suggestions = products.getIndexSuggestions();

      // Category should not be suggested (already indexed)
      expect(suggestions.find(s => s.attribute === 'category')).toBeUndefined();
    });

    it('resets statistics on demand', () => {
      const products = new IndexedLWWMap<string, Product>(hlc);

      products.set('p1', {
        id: 'p1',
        name: 'Product 1',
        category: 'cat1',
        price: 100,
        inStock: true,
        description: 'Desc',
      });

      products.query({ type: 'eq', attribute: 'category', value: 'cat1' });
      expect(products.getQueryStatistics().length).toBeGreaterThan(0);

      products.resetQueryStatistics();
      expect(products.getQueryStatistics()).toHaveLength(0);
    });
  });

  describe('Auto-Indexing (Development Mode)', () => {
    it('automatically creates indexes after threshold', () => {
      const createdIndexes: Array<{ attr: string; type: string }> = [];

      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: {
            enabled: true,
            threshold: 5,
            maxIndexes: 10,
            onIndexCreated: (attr, type) => {
              createdIndexes.push({ attr, type });
            },
          },
        },
      });

      // Register attribute for auto-indexing
      products.registerAttribute(simpleAttribute<Product, string>('category', p => p.category));

      // Add data
      for (let i = 0; i < 50; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: true,
          description: `Desc ${i}`,
        });
      }

      // Queries 1-4: No index yet
      for (let i = 0; i < 4; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }
      expect(products.hasIndexOn('category')).toBe(false);

      // Query 5: Triggers auto-index creation
      products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      expect(products.hasIndexOn('category')).toBe(true);
      expect(createdIndexes).toHaveLength(1);
      expect(createdIndexes[0]).toEqual({ attr: 'category', type: 'hash' });
    });

    it('creates appropriate index type for different query types', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: { enabled: true, threshold: 3 },
        },
      });

      // Register attributes
      products.registerAttribute(simpleAttribute<Product, string>('category', p => p.category));
      products.registerAttribute(simpleAttribute<Product, number>('price', p => p.price));

      // Add data
      for (let i = 0; i < 50; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: true,
          description: `Desc ${i}`,
        });
      }

      // Execute equality queries on category (should create HashIndex)
      for (let i = 0; i < 3; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      // Execute range queries on price (should create NavigableIndex)
      for (let i = 0; i < 3; i++) {
        products.query({ type: 'gt', attribute: 'price', value: 200 });
      }

      expect(products.hasIndexOn('category')).toBe(true);
      expect(products.hasIndexOn('price')).toBe(true);

      // Verify index types
      const indexes = products.getIndexes();
      const categoryIndex = indexes.find(i => i.attribute.name === 'category');
      const priceIndex = indexes.find(i => i.attribute.name === 'price');

      expect(categoryIndex?.type).toBe('hash');
      expect(priceIndex?.type).toBe('navigable');
    });

    it('respects maxIndexes limit', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: { enabled: true, threshold: 2, maxIndexes: 2 },
        },
      });

      // Register multiple attributes
      products.registerAttribute(simpleAttribute<Product, string>('category', p => p.category));
      products.registerAttribute(simpleAttribute<Product, number>('price', p => p.price));
      products.registerAttribute(simpleAttribute<Product, string>('name', p => p.name));

      // Add data
      products.set('p1', {
        id: 'p1',
        name: 'Product 1',
        category: 'cat1',
        price: 100,
        inStock: true,
        description: 'Desc',
      });

      // Create 2 indexes
      products.query({ type: 'eq', attribute: 'category', value: 'cat1' });
      products.query({ type: 'eq', attribute: 'category', value: 'cat1' });
      products.query({ type: 'gt', attribute: 'price', value: 50 });
      products.query({ type: 'gt', attribute: 'price', value: 50 });

      expect(products.hasIndexOn('category')).toBe(true);
      expect(products.hasIndexOn('price')).toBe(true);

      // Try to create 3rd index (should be rejected)
      products.query({ type: 'eq', attribute: 'name', value: 'Product 1' });
      products.query({ type: 'eq', attribute: 'name', value: 'Product 1' });

      expect(products.hasIndexOn('name')).toBe(false);
    });

    it('requires attribute registration for auto-indexing', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: { enabled: true, threshold: 2 },
        },
      });

      // Don't register attribute

      products.set('p1', {
        id: 'p1',
        name: 'Product 1',
        category: 'cat1',
        price: 100,
        inStock: true,
        description: 'Desc',
      });

      // Execute queries
      for (let i = 0; i < 5; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat1' });
      }

      // Should not create index without registration
      expect(products.hasIndexOn('category')).toBe(false);
    });

    it('reports auto-indexing status', () => {
      const productsWithAuto = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: { enabled: true, threshold: 5 },
        },
      });

      const productsWithoutAuto = new IndexedLWWMap<string, Product>(hlc);

      expect(productsWithAuto.isAutoIndexingEnabled()).toBe(true);
      expect(productsWithoutAuto.isAutoIndexingEnabled()).toBe(false);
    });
  });

  describe('Default Indexing Strategy', () => {
    it('automatically indexes scalar fields on first set', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        defaultIndexing: 'scalar',
      });

      // First set triggers default indexing
      products.set('p1', {
        id: 'p1',
        name: 'Product 1',
        category: 'electronics',
        price: 99.99,
        inStock: true,
        description: 'Short desc',
      });

      // Should have indexes on scalar fields
      expect(products.hasIndexOn('id')).toBe(true);
      expect(products.hasIndexOn('category')).toBe(true);
      expect(products.hasIndexOn('price')).toBe(true);
      expect(products.hasIndexOn('inStock')).toBe(true);

      // Long description should be skipped
      // (would need longer text to trigger skip)
    });
  });

  describe('Combined Advisor + Auto-Index', () => {
    it('works together: advisor tracks, auto-index creates', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          advisor: { enabled: true, minQueryCount: 3 },
          autoIndex: { enabled: true, threshold: 5 },
        },
      });

      products.registerAttribute(simpleAttribute<Product, string>('category', p => p.category));

      // Add data
      for (let i = 0; i < 50; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 5}`,
          price: i * 10,
          inStock: true,
          description: `Desc ${i}`,
        });
      }

      // 3 queries: Advisor should suggest, but no auto-index yet
      for (let i = 0; i < 3; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      const suggestions = products.getIndexSuggestions({ minQueryCount: 3, minAverageCost: 0 });
      expect(suggestions.find(s => s.attribute === 'category')).toBeDefined();
      expect(products.hasIndexOn('category')).toBe(false);

      // 2 more queries: Auto-index should create
      for (let i = 0; i < 2; i++) {
        products.query({ type: 'eq', attribute: 'category', value: 'cat0' });
      }

      expect(products.hasIndexOn('category')).toBe(true);
    });
  });

  describe('Query Performance Improvement', () => {
    it('shows performance improvement after auto-indexing', () => {
      const products = new IndexedLWWMap<string, Product>(hlc, {
        adaptiveIndexing: {
          autoIndex: { enabled: true, threshold: 5 },
        },
      });

      products.registerAttribute(simpleAttribute<Product, string>('category', p => p.category));

      // Add significant data
      for (let i = 0; i < 10000; i++) {
        products.set(`p${i}`, {
          id: `p${i}`,
          name: `Product ${i}`,
          category: `cat${i % 100}`,
          price: i * 0.01,
          inStock: i % 2 === 0,
          description: `Description for product ${i}`,
        });
      }

      // Measure queries before index
      const beforeTimes: number[] = [];
      for (let i = 0; i < 4; i++) {
        const start = performance.now();
        products.query({ type: 'eq', attribute: 'category', value: 'cat50' });
        beforeTimes.push(performance.now() - start);
      }

      // 5th query triggers auto-index
      products.query({ type: 'eq', attribute: 'category', value: 'cat50' });

      // Measure queries after index
      const afterTimes: number[] = [];
      for (let i = 0; i < 5; i++) {
        const start = performance.now();
        products.query({ type: 'eq', attribute: 'category', value: 'cat50' });
        afterTimes.push(performance.now() - start);
      }

      const avgBefore = beforeTimes.reduce((a, b) => a + b) / beforeTimes.length;
      const avgAfter = afterTimes.reduce((a, b) => a + b) / afterTimes.length;

      // After indexing should be significantly faster
      // Note: In small datasets the difference might not be dramatic
      expect(avgAfter).toBeLessThan(avgBefore);
    });
  });
});
