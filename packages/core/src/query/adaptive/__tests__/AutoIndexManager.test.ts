/**
 * Tests for AutoIndexManager (Phase 8.02.3)
 */

import { AutoIndexManager } from '../AutoIndexManager';
import { QueryPatternTracker } from '../QueryPatternTracker';
import { IndexAdvisor } from '../IndexAdvisor';
import type { IndexableMap } from '../AutoIndexManager';
import type { Attribute } from '../../Attribute';
import { simpleAttribute } from '../../Attribute';

// Mock IndexableMap for testing
class MockIndexableMap<V> implements IndexableMap<string, V> {
  private indexes: Map<string, { attribute: { name: string }; type: string }> = new Map();

  getIndexes() {
    return Array.from(this.indexes.values());
  }

  hasIndexOn(attributeName: string): boolean {
    return this.indexes.has(attributeName);
  }

  addHashIndex<A>(attribute: Attribute<V, A>): void {
    this.indexes.set(attribute.name, { attribute: { name: attribute.name }, type: 'hash' });
  }

  addNavigableIndex<A extends string | number>(attribute: Attribute<V, A>): void {
    this.indexes.set(attribute.name, { attribute: { name: attribute.name }, type: 'navigable' });
  }

  addInvertedIndex<A extends string>(attribute: Attribute<V, A>): void {
    this.indexes.set(attribute.name, { attribute: { name: attribute.name }, type: 'inverted' });
  }

  // Test helper
  clearIndexes(): void {
    this.indexes.clear();
  }
}

interface Product {
  id: string;
  category: string;
  price: number;
  description: string;
}

describe('AutoIndexManager', () => {
  let tracker: QueryPatternTracker;
  let advisor: IndexAdvisor;
  let map: MockIndexableMap<Product>;
  let createdIndexes: Array<{ attribute: string; indexType: string }>;
  let manager: AutoIndexManager<string, Product>;

  beforeEach(() => {
    tracker = new QueryPatternTracker();
    advisor = new IndexAdvisor(tracker);
    map = new MockIndexableMap();
    createdIndexes = [];

    manager = new AutoIndexManager(tracker, advisor, {
      enabled: true,
      threshold: 5,
      maxIndexes: 10,
      onIndexCreated: (attribute, indexType) => {
        createdIndexes.push({ attribute, indexType });
      },
    });
    manager.setMap(map);
  });

  describe('attribute registration', () => {
    it('registers attributes', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      expect(manager.hasAttribute('category')).toBe(true);
    });

    it('unregisters attributes', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);
      manager.unregisterAttribute('category');

      expect(manager.hasAttribute('category')).toBe(false);
    });

    it('returns registered attribute names', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', p => p.category);
      const priceAttr = simpleAttribute<Product, number>('price', p => p.price);

      manager.registerAttribute(categoryAttr);
      manager.registerAttribute(priceAttr);

      const names = manager.getRegisteredAttributeNames();
      expect(names).toContain('category');
      expect(names).toContain('price');
    });

    it('allows restricting index types for attribute', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr, ['hash']); // Only allow hash

      // Simulate queries
      for (let i = 0; i < 5; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      expect(map.hasIndexOn('category')).toBe(true);
    });
  });

  describe('onQueryExecuted', () => {
    it('does nothing when disabled', () => {
      const disabledManager = new AutoIndexManager(tracker, advisor, {
        enabled: false,
        threshold: 5,
      });
      disabledManager.setMap(map);

      const attr = simpleAttribute<Product, string>('category', p => p.category);
      disabledManager.registerAttribute(attr);

      for (let i = 0; i < 10; i++) {
        disabledManager.onQueryExecuted('category', 'eq');
      }

      expect(map.hasIndexOn('category')).toBe(false);
    });

    it('creates hash index after threshold for equality queries', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      // Queries 1-4: No index yet
      for (let i = 0; i < 4; i++) {
        manager.onQueryExecuted('category', 'eq');
        expect(map.hasIndexOn('category')).toBe(false);
      }

      // Query 5: Threshold reached, index created
      manager.onQueryExecuted('category', 'eq');
      expect(map.hasIndexOn('category')).toBe(true);
      expect(createdIndexes).toHaveLength(1);
      expect(createdIndexes[0]).toEqual({ attribute: 'category', indexType: 'hash' });
    });

    it('creates navigable index for range queries', () => {
      const attr = simpleAttribute<Product, number>('price', p => p.price);
      manager.registerAttribute(attr);

      for (let i = 0; i < 5; i++) {
        manager.onQueryExecuted('price', 'gt');
      }

      expect(map.hasIndexOn('price')).toBe(true);
      expect(createdIndexes[0].indexType).toBe('navigable');
    });

    it('creates inverted index for text search queries', () => {
      const attr = simpleAttribute<Product, string>('description', p => p.description);
      manager.registerAttribute(attr);

      for (let i = 0; i < 5; i++) {
        manager.onQueryExecuted('description', 'contains');
      }

      expect(map.hasIndexOn('description')).toBe(true);
      expect(createdIndexes[0].indexType).toBe('inverted');
    });

    it('does not create index for unregistered attribute', () => {
      // Don't register 'category'

      for (let i = 0; i < 10; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      expect(map.hasIndexOn('category')).toBe(false);
    });

    it('does not create duplicate indexes', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      // Create first index
      for (let i = 0; i < 5; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      // More queries shouldn't create another index
      for (let i = 0; i < 10; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      expect(createdIndexes).toHaveLength(1);
    });

    it('respects maxIndexes limit', () => {
      // Create manager with low limit
      const limitedManager = new AutoIndexManager(tracker, advisor, {
        enabled: true,
        threshold: 2,
        maxIndexes: 2,
      });
      limitedManager.setMap(map);

      // Register three attributes
      const attrs = ['a', 'b', 'c'].map(name =>
        simpleAttribute<Product, string>(name, () => name)
      );
      attrs.forEach(attr => limitedManager.registerAttribute(attr));

      // Create indexes for a and b
      limitedManager.onQueryExecuted('a', 'eq');
      limitedManager.onQueryExecuted('a', 'eq');
      limitedManager.onQueryExecuted('b', 'eq');
      limitedManager.onQueryExecuted('b', 'eq');

      expect(map.hasIndexOn('a')).toBe(true);
      expect(map.hasIndexOn('b')).toBe(true);

      // c should not get an index (limit reached)
      limitedManager.onQueryExecuted('c', 'eq');
      limitedManager.onQueryExecuted('c', 'eq');

      expect(map.hasIndexOn('c')).toBe(false);
    });

    it('skips already indexed attributes', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      // Manually add index
      map.addHashIndex(attr);

      for (let i = 0; i < 10; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      // Should not call callback
      expect(createdIndexes).toHaveLength(0);
    });
  });

  describe('isAtLimit', () => {
    it('returns true when max indexes reached', () => {
      const limitedManager = new AutoIndexManager(tracker, advisor, {
        enabled: true,
        threshold: 1,
        maxIndexes: 2,
      });
      limitedManager.setMap(map);

      expect(limitedManager.isAtLimit()).toBe(false);

      // Add two indexes
      const attr1 = simpleAttribute<Product, string>('a', () => 'a');
      const attr2 = simpleAttribute<Product, string>('b', () => 'b');
      map.addHashIndex(attr1);
      map.addHashIndex(attr2);

      expect(limitedManager.isAtLimit()).toBe(true);
    });
  });

  describe('getAutoCreatedIndexCount', () => {
    it('returns count of auto-created indexes', () => {
      expect(manager.getAutoCreatedIndexCount()).toBe(0);

      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      for (let i = 0; i < 5; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      expect(manager.getAutoCreatedIndexCount()).toBe(1);
    });
  });

  describe('getRemainingCapacity', () => {
    it('returns remaining index slots', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      map.addHashIndex(attr);

      expect(manager.getRemainingCapacity()).toBe(9); // 10 - 1
    });
  });

  describe('resetCounts', () => {
    it('clears query counts', () => {
      const attr = simpleAttribute<Product, string>('category', p => p.category);
      manager.registerAttribute(attr);

      // 4 queries, not enough for threshold
      for (let i = 0; i < 4; i++) {
        manager.onQueryExecuted('category', 'eq');
      }

      manager.resetCounts();

      // Need full 5 queries again
      for (let i = 0; i < 4; i++) {
        manager.onQueryExecuted('category', 'eq');
      }
      expect(map.hasIndexOn('category')).toBe(false);

      manager.onQueryExecuted('category', 'eq');
      expect(map.hasIndexOn('category')).toBe(true);
    });
  });

  describe('getConfig', () => {
    it('returns current configuration', () => {
      const config = manager.getConfig();
      expect(config.enabled).toBe(true);
      expect(config.threshold).toBe(5);
      expect(config.maxIndexes).toBe(10);
    });
  });

  describe('updateConfig', () => {
    it('updates configuration at runtime', () => {
      manager.updateConfig({ threshold: 3, enabled: false });

      const config = manager.getConfig();
      expect(config.threshold).toBe(3);
      expect(config.enabled).toBe(false);
    });
  });
});
