/**
 * Tests for DefaultIndexingStrategy (Phase 8.02.4)
 */

import { DefaultIndexingStrategy } from '../DefaultIndexingStrategy';
import type { DefaultIndexableMap, FieldIndexRecommendation } from '../DefaultIndexingStrategy';
import type { Attribute } from '../../Attribute';

interface Product {
  id: string;
  name: string;
  category: string;
  price: number;
  inStock: boolean;
  description: string;
  tags: string[];
  metadata: {
    createdAt: string;
    rating: number;
  };
}

// Mock map for testing
class MockDefaultIndexableMap<V> implements DefaultIndexableMap<V> {
  private indexes: Map<string, string> = new Map();

  addHashIndex<A>(attribute: Attribute<V, A>): void {
    this.indexes.set(attribute.name, 'hash');
  }

  addNavigableIndex<A extends string | number>(attribute: Attribute<V, A>): void {
    this.indexes.set(attribute.name, 'navigable');
  }

  hasIndexOn(attributeName: string): boolean {
    return this.indexes.has(attributeName);
  }

  getIndexType(attributeName: string): string | undefined {
    return this.indexes.get(attributeName);
  }

  getIndexedAttributes(): string[] {
    return Array.from(this.indexes.keys());
  }
}

describe('DefaultIndexingStrategy', () => {
  describe('with "none" strategy', () => {
    it('does not apply any indexes', () => {
      const strategy = new DefaultIndexingStrategy<Product>('none');
      const map = new MockDefaultIndexableMap<Product>();

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'A nice widget',
        tags: ['popular', 'sale'],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.getIndexedAttributes()).toHaveLength(0);
    });
  });

  describe('with "scalar" strategy', () => {
    let strategy: DefaultIndexingStrategy<Product>;
    let map: MockDefaultIndexableMap<Product>;

    beforeEach(() => {
      strategy = new DefaultIndexingStrategy<Product>('scalar');
      map = new MockDefaultIndexableMap<Product>();
    });

    it('indexes scalar fields', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short desc',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      // Should index top-level scalars
      expect(map.hasIndexOn('id')).toBe(true);
      expect(map.hasIndexOn('name')).toBe(true);
      expect(map.hasIndexOn('category')).toBe(true);
      expect(map.hasIndexOn('price')).toBe(true);
      expect(map.hasIndexOn('inStock')).toBe(true);
    });

    it('uses hash index for strings', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.getIndexType('category')).toBe('hash');
    });

    it('uses navigable index for numbers', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.getIndexType('price')).toBe('navigable');
    });

    it('uses hash index for booleans', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.getIndexType('inStock')).toBe('hash');
    });

    it('uses hash index for ID fields', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.getIndexType('id')).toBe('hash');
    });

    it('does not index arrays', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: ['a', 'b'],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.hasIndexOn('tags')).toBe(false);
    });

    it('does not index objects', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.hasIndexOn('metadata')).toBe(false);
    });

    it('does not index nested fields', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      // metadata.createdAt and metadata.rating should not be indexed
      expect(map.hasIndexOn('metadata.createdAt')).toBe(false);
      expect(map.hasIndexOn('metadata.rating')).toBe(false);
    });

    it('skips description-like fields', () => {
      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'A very long description that goes on and on...',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.hasIndexOn('description')).toBe(false);
    });

    it('applies only once', () => {
      const sample1: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample1);
      expect(strategy.isApplied()).toBe(true);

      // Clear and try again
      const map2 = new MockDefaultIndexableMap<Product>();
      strategy.applyToMap(map2, sample1);

      // Should not apply again
      expect(map2.getIndexedAttributes()).toHaveLength(0);
    });

    it('skips existing indexes', () => {
      // Pre-add an index
      map.addHashIndex({ name: 'category', type: 'simple', getValue: () => '', getValues: () => [] });

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      // Should not throw or fail
      expect(map.hasIndexOn('category')).toBe(true);
    });
  });

  describe('with "all" strategy', () => {
    it('includes nested fields', () => {
      const strategy = new DefaultIndexingStrategy<Product>('all');
      const map = new MockDefaultIndexableMap<Product>();

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      expect(map.hasIndexOn('metadata.createdAt')).toBe(true);
      expect(map.hasIndexOn('metadata.rating')).toBe(true);
    });

    it('uses appropriate index types for nested fields', () => {
      const strategy = new DefaultIndexingStrategy<Product>('all');
      const map = new MockDefaultIndexableMap<Product>();

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      strategy.applyToMap(map, sample);

      // rating is a number - should use navigable
      expect(map.getIndexType('metadata.rating')).toBe('navigable');
    });
  });

  describe('analyzeAndRecommend', () => {
    it('returns recommendations without applying', () => {
      const strategy = new DefaultIndexingStrategy<Product>('scalar');

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      const recommendations = strategy.analyzeAndRecommend(sample);

      expect(recommendations.length).toBeGreaterThan(0);
      expect(recommendations.find(r => r.field === 'id')).toBeDefined();
      expect(recommendations.find(r => r.field === 'price')).toBeDefined();

      // Should not apply to any map
      expect(strategy.isApplied()).toBe(false);
    });

    it('includes reasons for recommendations', () => {
      const strategy = new DefaultIndexingStrategy<Product>('scalar');

      const sample: Product = {
        id: '1',
        name: 'Widget',
        category: 'electronics',
        price: 29.99,
        inStock: true,
        description: 'Short',
        tags: [],
        metadata: { createdAt: '2024-01-01', rating: 4.5 },
      };

      const recommendations = strategy.analyzeAndRecommend(sample);

      for (const rec of recommendations) {
        expect(rec.reason).toBeDefined();
        expect(rec.reason.length).toBeGreaterThan(0);
      }
    });
  });

  describe('date-like fields', () => {
    interface Event {
      id: string;
      title: string;
      created_at: string;
      eventDate: string;
      startTime: string;
    }

    it('uses navigable index for date-like fields', () => {
      const strategy = new DefaultIndexingStrategy<Event>('scalar');
      const map = new MockDefaultIndexableMap<Event>();

      const sample: Event = {
        id: '1',
        title: 'Event',
        created_at: '2024-01-01T10:00:00Z',
        eventDate: '2024-02-01',
        startTime: '10:00:00',
      };

      strategy.applyToMap(map, sample);

      // Fields with 'date' or 'time' in name should use navigable
      expect(map.getIndexType('eventDate')).toBe('navigable');
      expect(map.getIndexType('startTime')).toBe('navigable');
      // Fields ending with _at suffix should use navigable
      expect(map.getIndexType('created_at')).toBe('navigable');
    });
  });
});
