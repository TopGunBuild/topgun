import { NavigableIndex } from '../../../query/indexes/NavigableIndex';
import { simpleAttribute, multiAttribute } from '../../../query/Attribute';
import { LazyResultSet } from '../../../query/resultset/LazyResultSet';

interface Product {
  id: string;
  name: string;
  price: number;
  rating: number;
  category: string;
  tags: string[];
}

const priceAttr = simpleAttribute<Product, number>('price', (p) => p.price);
const ratingAttr = simpleAttribute<Product, number>('rating', (p) => p.rating);
const categoryAttr = simpleAttribute<Product, string>(
  'category',
  (p) => p.category
);
const tagsAttr = multiAttribute<Product, string>('tags', (p) => p.tags);

describe('NavigableIndex', () => {
  describe('basic properties', () => {
    it('should have type navigable', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.type).toBe('navigable');
    });

    it('should return correct retrieval cost (40)', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.getRetrievalCost()).toBe(40);
    });

    it('should support equality queries', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.supportsQuery('equal')).toBe(true);
      expect(index.supportsQuery('in')).toBe(true);
      expect(index.supportsQuery('has')).toBe(true);
    });

    it('should support range queries', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.supportsQuery('gt')).toBe(true);
      expect(index.supportsQuery('gte')).toBe(true);
      expect(index.supportsQuery('lt')).toBe(true);
      expect(index.supportsQuery('lte')).toBe(true);
      expect(index.supportsQuery('between')).toBe(true);
    });

    it('should expose attribute', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.attribute).toBe(priceAttr);
    });
  });

  describe('equality queries', () => {
    let index: NavigableIndex<string, Product, number>;
    const products: Product[] = [
      { id: '1', name: 'A', price: 100, rating: 4.5, category: 'electronics', tags: [] },
      { id: '2', name: 'B', price: 200, rating: 4.0, category: 'clothing', tags: [] },
      { id: '3', name: 'C', price: 100, rating: 3.5, category: 'electronics', tags: [] },
      { id: '4', name: 'D', price: 300, rating: 5.0, category: 'food', tags: [] },
    ];

    beforeEach(() => {
      index = new NavigableIndex(priceAttr);
      products.forEach((p) => index.add(p.id, p));
    });

    it('should retrieve by equal query', () => {
      const result = index.retrieve({ type: 'equal', value: 100 });
      expect([...result].sort()).toEqual(['1', '3']);
    });

    it('should retrieve by in query', () => {
      const result = index.retrieve({ type: 'in', values: [100, 300] });
      expect([...result].sort()).toEqual(['1', '3', '4']);
    });

    it('should retrieve by has query', () => {
      const result = index.retrieve({ type: 'has' });
      expect([...result].sort()).toEqual(['1', '2', '3', '4']);
    });

    it('should return empty for non-existent value', () => {
      const result = index.retrieve({ type: 'equal', value: 999 });
      expect([...result]).toEqual([]);
      expect(result.isEmpty()).toBe(true);
    });
  });

  describe('range queries - numbers', () => {
    let index: NavigableIndex<string, Product, number>;
    const products: Product[] = [
      { id: '1', name: 'A', price: 100, rating: 4.5, category: 'a', tags: [] },
      { id: '2', name: 'B', price: 200, rating: 4.0, category: 'b', tags: [] },
      { id: '3', name: 'C', price: 150, rating: 3.5, category: 'c', tags: [] },
      { id: '4', name: 'D', price: 300, rating: 5.0, category: 'd', tags: [] },
      { id: '5', name: 'E', price: 250, rating: 4.2, category: 'e', tags: [] },
    ];

    beforeEach(() => {
      index = new NavigableIndex(priceAttr);
      products.forEach((p) => index.add(p.id, p));
    });

    it('should retrieve gt (greater than)', () => {
      const result = index.retrieve({ type: 'gt', value: 200 });
      expect([...result].sort()).toEqual(['4', '5']); // 250, 300
    });

    it('should retrieve gte (greater than or equal)', () => {
      const result = index.retrieve({ type: 'gte', value: 200 });
      expect([...result].sort()).toEqual(['2', '4', '5']); // 200, 250, 300
    });

    it('should retrieve lt (less than)', () => {
      const result = index.retrieve({ type: 'lt', value: 200 });
      expect([...result].sort()).toEqual(['1', '3']); // 100, 150
    });

    it('should retrieve lte (less than or equal)', () => {
      const result = index.retrieve({ type: 'lte', value: 200 });
      expect([...result].sort()).toEqual(['1', '2', '3']); // 100, 150, 200
    });

    it('should retrieve between [from, to) - default', () => {
      const result = index.retrieve({
        type: 'between',
        from: 100,
        to: 250,
        // fromInclusive: true (default)
        // toInclusive: false (default)
      });
      expect([...result].sort()).toEqual(['1', '2', '3']); // 100, 150, 200 (not 250)
    });

    it('should retrieve between [from, to]', () => {
      const result = index.retrieve({
        type: 'between',
        from: 100,
        to: 250,
        fromInclusive: true,
        toInclusive: true,
      });
      expect([...result].sort()).toEqual(['1', '2', '3', '5']); // 100, 150, 200, 250
    });

    it('should retrieve between (from, to)', () => {
      const result = index.retrieve({
        type: 'between',
        from: 100,
        to: 250,
        fromInclusive: false,
        toInclusive: false,
      });
      expect([...result].sort()).toEqual(['2', '3']); // 150, 200 (not 100, not 250)
    });

    it('should retrieve between (from, to]', () => {
      const result = index.retrieve({
        type: 'between',
        from: 100,
        to: 250,
        fromInclusive: false,
        toInclusive: true,
      });
      expect([...result].sort()).toEqual(['2', '3', '5']); // 150, 200, 250 (not 100)
    });
  });

  describe('range queries - strings', () => {
    let index: NavigableIndex<string, Product, string>;
    const products: Product[] = [
      { id: '1', name: 'Apple', price: 1, rating: 1, category: 'apple', tags: [] },
      { id: '2', name: 'Banana', price: 2, rating: 2, category: 'banana', tags: [] },
      { id: '3', name: 'Cherry', price: 3, rating: 3, category: 'cherry', tags: [] },
      { id: '4', name: 'Date', price: 4, rating: 4, category: 'date', tags: [] },
      { id: '5', name: 'Elderberry', price: 5, rating: 5, category: 'elderberry', tags: [] },
    ];

    beforeEach(() => {
      index = new NavigableIndex(categoryAttr);
      products.forEach((p) => index.add(p.id, p));
    });

    it('should retrieve lexicographically greater', () => {
      const result = index.retrieve({ type: 'gt', value: 'cherry' });
      expect([...result].sort()).toEqual(['4', '5']); // date, elderberry
    });

    it('should retrieve lexicographically less', () => {
      const result = index.retrieve({ type: 'lt', value: 'cherry' });
      expect([...result].sort()).toEqual(['1', '2']); // apple, banana
    });

    it('should retrieve between with strings', () => {
      const result = index.retrieve({
        type: 'between',
        from: 'banana',
        to: 'elderberry',
        fromInclusive: true,
        toInclusive: false,
      });
      expect([...result].sort()).toEqual(['2', '3', '4']); // banana, cherry, date
    });

    it('should use lexicographic ordering', () => {
      // Verify order: apple < banana < cherry < date < elderberry
      const result = index.retrieve({ type: 'gte', value: 'cherry' });
      expect([...result].sort()).toEqual(['3', '4', '5']);
    });
  });

  describe('edge cases', () => {
    it('should return empty for gt when no values greater', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: 'A', price: 100, rating: 1, category: '', tags: [] });

      const result = index.retrieve({ type: 'gt', value: 100 });
      expect([...result]).toEqual([]);
    });

    it('should return empty for lt when no values less', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: 'A', price: 100, rating: 1, category: '', tags: [] });

      const result = index.retrieve({ type: 'lt', value: 100 });
      expect([...result]).toEqual([]);
    });

    it('should return empty for between with invalid range', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: 'A', price: 100, rating: 1, category: '', tags: [] });
      index.add('2', { id: '2', name: 'B', price: 200, rating: 2, category: '', tags: [] });

      // from > to
      const result = index.retrieve({
        type: 'between',
        from: 300,
        to: 100,
      });
      expect([...result]).toEqual([]);
    });

    it('should handle single-element ranges', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: 'A', price: 100, rating: 1, category: '', tags: [] });
      index.add('2', { id: '2', name: 'B', price: 100, rating: 2, category: '', tags: [] });

      // Range containing only 100
      const result = index.retrieve({
        type: 'between',
        from: 100,
        to: 100,
        fromInclusive: true,
        toInclusive: true,
      });
      expect([...result].sort()).toEqual(['1', '2']);
    });

    it('should handle empty index', () => {
      const index = new NavigableIndex(priceAttr);

      expect([...index.retrieve({ type: 'equal', value: 100 })]).toEqual([]);
      expect([...index.retrieve({ type: 'gt', value: 100 })]).toEqual([]);
      expect([...index.retrieve({ type: 'lt', value: 100 })]).toEqual([]);
      expect([...index.retrieve({ type: 'has' })]).toEqual([]);
    });

    it('should throw for unsupported query type', () => {
      const index = new NavigableIndex(priceAttr);
      expect(() => {
        index.retrieve({ type: 'contains' as 'equal', value: 100 });
      }).toThrow('NavigableIndex does not support query type: contains');
    });
  });

  describe('lazy evaluation', () => {
    it('should not materialize until iterated', () => {
      const index = new NavigableIndex(priceAttr);
      for (let i = 0; i < 100; i++) {
        index.add(String(i), { id: String(i), name: '', price: i, rating: 0, category: '', tags: [] });
      }

      const result = index.retrieve({ type: 'gt', value: 50 });
      expect(result).toBeInstanceOf(LazyResultSet);
      expect((result as LazyResultSet<string>).isMaterialized()).toBe(false);
    });

    it('should cache after first iteration via toArray', () => {
      const index = new NavigableIndex(priceAttr);
      for (let i = 0; i < 10; i++) {
        index.add(String(i), { id: String(i), name: '', price: i * 10, rating: 0, category: '', tags: [] });
      }

      const result = index.retrieve({ type: 'gte', value: 50 }) as LazyResultSet<string>;

      expect(result.isMaterialized()).toBe(false);
      result.toArray();
      expect(result.isMaterialized()).toBe(true);
    });

    it('should provide estimated merge cost before materialization', () => {
      const index = new NavigableIndex(priceAttr);
      for (let i = 0; i < 100; i++) {
        index.add(String(i), { id: String(i), name: '', price: i, rating: 0, category: '', tags: [] });
      }

      const result = index.retrieve({ type: 'gt', value: 50 }) as LazyResultSet<string>;

      // Estimated size should be allKeys.size / 2 = 50
      expect(result.getMergeCost()).toBe(50);
      expect(result.isMaterialized()).toBe(false);
    });

    it('should report actual size after materialization', () => {
      const index = new NavigableIndex(priceAttr);
      for (let i = 0; i < 100; i++) {
        index.add(String(i), { id: String(i), name: '', price: i, rating: 0, category: '', tags: [] });
      }

      const result = index.retrieve({ type: 'gt', value: 50 }) as LazyResultSet<string>;

      result.toArray();
      // Actual size: 51-99 = 49 items
      expect(result.getMergeCost()).toBe(49);
    });

    it('should use SetResultSet for equality queries (not lazy)', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 100, rating: 0, category: '', tags: [] });

      const result = index.retrieve({ type: 'equal', value: 100 });
      expect(result).not.toBeInstanceOf(LazyResultSet);
    });
  });

  describe('add/remove/update', () => {
    it('should add record to index', () => {
      const index = new NavigableIndex(priceAttr);
      const product: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] };

      index.add('1', product);

      expect([...index.retrieve({ type: 'equal', value: 100 })]).toEqual(['1']);
      expect(index.getMinValue()).toBe(100);
      expect(index.getMaxValue()).toBe(100);
    });

    it('should remove record from index', () => {
      const index = new NavigableIndex(priceAttr);
      const product: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] };

      index.add('1', product);
      index.remove('1', product);

      expect([...index.retrieve({ type: 'equal', value: 100 })]).toEqual([]);
    });

    it('should handle multi-value attributes', () => {
      const index = new NavigableIndex(tagsAttr);
      const product: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: ['tag1', 'tag2'] };

      index.add('1', product);

      expect([...index.retrieve({ type: 'equal', value: 'tag1' })]).toEqual(['1']);
      expect([...index.retrieve({ type: 'equal', value: 'tag2' })]).toEqual(['1']);
    });

    it('should clean empty buckets on remove', () => {
      const index = new NavigableIndex(priceAttr);
      const product: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] };

      index.add('1', product);
      expect(index.getStats().distinctValues).toBe(1);

      index.remove('1', product);
      expect(index.getStats().distinctValues).toBe(0);
    });

    it('should not add record with empty values', () => {
      const index = new NavigableIndex(tagsAttr);
      const product: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] };

      index.add('1', product);
      expect(index.getStats().totalEntries).toBe(0);
    });

    it('should update record correctly', () => {
      const index = new NavigableIndex(priceAttr);
      const product1: Product = { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] };
      const product2: Product = { id: '1', name: 'A', price: 200, rating: 0, category: '', tags: [] };

      index.add('1', product1);
      index.update('1', product1, product2);

      expect([...index.retrieve({ type: 'equal', value: 100 })]).toEqual([]);
      expect([...index.retrieve({ type: 'equal', value: 200 })]).toEqual(['1']);
    });

    it('should skip update if value unchanged', () => {
      const index = new NavigableIndex(priceAttr);
      const product1: Product = { id: '1', name: 'A', price: 100, rating: 1, category: '', tags: [] };
      const product2: Product = { id: '1', name: 'B', price: 100, rating: 2, category: '', tags: [] };

      index.add('1', product1);
      const statsBefore = index.getStats();

      index.update('1', product1, product2);

      expect(index.getStats()).toEqual(statsBefore);
    });
  });

  describe('custom comparator', () => {
    it('should use custom comparator for ordering', () => {
      // Reverse comparator
      const reverseComparator = (a: number, b: number) => b - a;
      const index = new NavigableIndex(priceAttr, reverseComparator);

      index.add('1', { id: '1', name: 'A', price: 100, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: 'B', price: 200, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: 'C', price: 300, rating: 0, category: '', tags: [] });

      // With reverse comparator, min and max are swapped
      expect(index.getMinValue()).toBe(300);
      expect(index.getMaxValue()).toBe(100);
    });

    it('should use custom comparator for range queries', () => {
      // Case-insensitive string comparator
      const caseInsensitiveComparator = (a: string, b: string) =>
        a.toLowerCase().localeCompare(b.toLowerCase());

      const index = new NavigableIndex(categoryAttr, caseInsensitiveComparator);

      index.add('1', { id: '1', name: 'A', price: 0, rating: 0, category: 'Apple', tags: [] });
      index.add('2', { id: '2', name: 'B', price: 0, rating: 0, category: 'BANANA', tags: [] });
      index.add('3', { id: '3', name: 'C', price: 0, rating: 0, category: 'cherry', tags: [] });

      // Should find 'BANANA' when searching for 'banana'
      const result = index.retrieve({ type: 'equal', value: 'BANANA' });
      expect([...result]).toEqual(['2']);

      // Range should work case-insensitively
      const rangeResult = index.retrieve({
        type: 'between',
        from: 'a',
        to: 'c',
        fromInclusive: true,
        toInclusive: false,
      });
      expect([...rangeResult].sort()).toEqual(['1', '2']); // Apple, BANANA
    });
  });

  describe('stats', () => {
    it('should report correct distinct values', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: '', price: 200, rating: 0, category: '', tags: [] });

      expect(index.getStats().distinctValues).toBe(2);
    });

    it('should report correct total entries', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: '', price: 200, rating: 0, category: '', tags: [] });

      expect(index.getStats().totalEntries).toBe(3);
    });

    it('should calculate avg entries per value', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: '', price: 200, rating: 0, category: '', tags: [] });

      expect(index.getStats().avgEntriesPerValue).toBe(1.5);
    });
  });

  describe('clear', () => {
    it('should clear all entries', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 200, rating: 0, category: '', tags: [] });

      index.clear();

      expect(index.getStats().distinctValues).toBe(0);
      expect(index.getStats().totalEntries).toBe(0);
      expect([...index.retrieve({ type: 'has' })]).toEqual([]);
    });
  });

  describe('performance', () => {
    it('should handle 100k records efficiently', () => {
      const index = new NavigableIndex(priceAttr);

      // Add 100k records
      for (let i = 0; i < 100000; i++) {
        index.add(String(i), {
          id: String(i),
          name: `Product ${i}`,
          price: i,
          rating: (i % 5) + 1,
          category: '',
          tags: [],
        });
      }

      // Verify stats
      expect(index.getStats().totalEntries).toBe(100000);

      // Range query should be fast
      const start = performance.now();
      const result = index.retrieve({ type: 'between', from: 50000, to: 60000 });
      const elapsed = performance.now() - start;

      // Should return 10000 records (50000-59999)
      expect(result.size()).toBe(10000);
      expect(elapsed).toBeLessThan(100); // Should be fast (< 100ms including materialization)
    });

    it('should maintain O(log N) retrieval time', () => {
      const index = new NavigableIndex(priceAttr);

      // Add 50k records
      for (let i = 0; i < 50000; i++) {
        index.add(String(i), {
          id: String(i),
          name: '',
          price: i,
          rating: 0,
          category: '',
          tags: [],
        });
      }

      // Multiple range queries should all be fast
      const times: number[] = [];
      const queries = [
        { type: 'gte' as const, value: 10000 },
        { type: 'lt' as const, value: 20000 },
        { type: 'between' as const, from: 25000, to: 30000 },
      ];

      for (const query of queries) {
        const start = performance.now();
        const result = index.retrieve(query);
        // Just iterate without materializing
        let count = 0;
        for (const _ of result) {
          count++;
        }
        times.push(performance.now() - start);
      }

      // All queries should complete in reasonable time
      for (const time of times) {
        expect(time).toBeLessThan(50);
      }
    });
  });

  describe('min/max value helpers', () => {
    it('should return min value', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 300, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: '', price: 200, rating: 0, category: '', tags: [] });

      expect(index.getMinValue()).toBe(100);
    });

    it('should return max value', () => {
      const index = new NavigableIndex(priceAttr);
      index.add('1', { id: '1', name: '', price: 300, rating: 0, category: '', tags: [] });
      index.add('2', { id: '2', name: '', price: 100, rating: 0, category: '', tags: [] });
      index.add('3', { id: '3', name: '', price: 200, rating: 0, category: '', tags: [] });

      expect(index.getMaxValue()).toBe(300);
    });

    it('should return undefined for empty index', () => {
      const index = new NavigableIndex(priceAttr);
      expect(index.getMinValue()).toBeUndefined();
      expect(index.getMaxValue()).toBeUndefined();
    });
  });
});
