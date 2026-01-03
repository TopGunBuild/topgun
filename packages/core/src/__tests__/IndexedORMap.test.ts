import { HLC } from '../HLC';
import { IndexedORMap } from '../IndexedORMap';
import { simpleAttribute } from '../query/Attribute';
import type { Query } from '../query/QueryTypes';

interface Product {
  name: string;
  category: string;
  price: number;
  inStock: boolean;
}

describe('IndexedORMap', () => {
  let hlc: HLC;
  let map: IndexedORMap<string, Product>;

  beforeEach(() => {
    hlc = new HLC('node1');
    map = new IndexedORMap<string, Product>(hlc);
  });

  describe('basic operations', () => {
    it('should work as a regular ORMap', () => {
      const product: Product = { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true };
      const record = map.add('product1', product);

      expect(record).toBeDefined();
      expect(record.value).toEqual(product);
      expect(map.get('product1')).toContainEqual(product);
    });

    it('should support multiple values per key', () => {
      const product1: Product = { name: 'Widget A', category: 'Electronics', price: 99.99, inStock: true };
      const product2: Product = { name: 'Widget B', category: 'Electronics', price: 149.99, inStock: true };

      map.add('widgets', product1);
      map.add('widgets', product2);

      const values = map.get('widgets');
      expect(values).toHaveLength(2);
      expect(values).toContainEqual(product1);
      expect(values).toContainEqual(product2);
    });

    it('should remove specific values', () => {
      const product1: Product = { name: 'Widget A', category: 'Electronics', price: 99.99, inStock: true };
      const product2: Product = { name: 'Widget B', category: 'Electronics', price: 149.99, inStock: true };

      map.add('widgets', product1);
      map.add('widgets', product2);
      map.remove('widgets', product1);

      const values = map.get('widgets');
      expect(values).toHaveLength(1);
      expect(values).toContainEqual(product2);
    });

    it('should clear all data', () => {
      map.add('product1', { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true });
      map.add('product2', { name: 'Gadget', category: 'Electronics', price: 199.99, inStock: false });

      map.clear();

      expect(map.size).toBe(0);
    });
  });

  describe('index management', () => {
    it('should create hash index', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      const index = map.addHashIndex(categoryAttr);

      expect(index).toBeDefined();
      expect(index.type).toBe('hash');
      expect(map.hasIndexOn('category')).toBe(true);
    });

    it('should create navigable index', () => {
      const priceAttr = simpleAttribute<Product, number>('price', (p) => p.price);
      const index = map.addNavigableIndex(priceAttr);

      expect(index).toBeDefined();
      expect(index.type).toBe('navigable');
      expect(map.hasIndexOn('price')).toBe(true);
    });

    it('should build index from existing data', () => {
      // Add data first
      map.add('product1', { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true });
      map.add('product2', { name: 'Gadget', category: 'Appliances', price: 199.99, inStock: true });

      // Then add index - it should build from existing data
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      map.addHashIndex(categoryAttr);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Widget');
    });

    it('should get all indexes', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      const priceAttr = simpleAttribute<Product, number>('price', (p) => p.price);

      map.addHashIndex(categoryAttr);
      map.addNavigableIndex(priceAttr);

      const indexes = map.getIndexes();
      expect(indexes).toHaveLength(2);
    });
  });

  describe('indexed queries', () => {
    beforeEach(() => {
      // Set up indexes
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      const priceAttr = simpleAttribute<Product, number>('price', (p) => p.price);

      map.addHashIndex(categoryAttr);
      map.addNavigableIndex(priceAttr);

      // Add data - multiple values per key to test ORMap behavior
      map.add('electronics', { name: 'Laptop', category: 'Electronics', price: 999.99, inStock: true });
      map.add('electronics', { name: 'Phone', category: 'Electronics', price: 699.99, inStock: true });
      map.add('appliances', { name: 'Blender', category: 'Appliances', price: 49.99, inStock: true });
      map.add('appliances', { name: 'Toaster', category: 'Appliances', price: 29.99, inStock: false });
      map.add('furniture', { name: 'Chair', category: 'Furniture', price: 149.99, inStock: true });
    });

    it('should use hash index for equal query', () => {
      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(2);
      const names = results.map((r) => r.value.name).sort();
      expect(names).toEqual(['Laptop', 'Phone']);
    });

    it('should use navigable index for range query', () => {
      const query: Query = { type: 'gte', attribute: 'price', value: 100 };
      const results = map.query(query);

      expect(results).toHaveLength(3);
      const names = results.map((r) => r.value.name).sort();
      expect(names).toEqual(['Chair', 'Laptop', 'Phone']);
    });

    it('should handle AND queries with multiple indexes', () => {
      const query: Query = {
        type: 'and',
        children: [
          { type: 'eq', attribute: 'category', value: 'Electronics' },
          { type: 'lte', attribute: 'price', value: 800 },
        ],
      };
      const results = map.query(query);

      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Phone');
    });

    it('should handle OR queries', () => {
      const query: Query = {
        type: 'or',
        children: [
          { type: 'eq', attribute: 'category', value: 'Electronics' },
          { type: 'eq', attribute: 'category', value: 'Furniture' },
        ],
      };
      const results = map.query(query);

      expect(results).toHaveLength(3);
      const names = results.map((r) => r.value.name).sort();
      expect(names).toEqual(['Chair', 'Laptop', 'Phone']);
    });

    it('should return query results with key and tag', () => {
      const query: Query = { type: 'eq', attribute: 'category', value: 'Appliances' };
      const results = map.query(query);

      expect(results).toHaveLength(2);
      for (const result of results) {
        expect(result.key).toBeDefined();
        expect(result.tag).toBeDefined();
        expect(result.value).toBeDefined();
      }
    });

    it('should return values only with queryValues', () => {
      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const values = map.queryValues(query);

      expect(values).toHaveLength(2);
      expect(values.every((p) => p.category === 'Electronics')).toBe(true);
    });

    it('should count matching records', () => {
      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const count = map.count(query);

      expect(count).toBe(2);
    });
  });

  describe('CRDT operations update indexes', () => {
    beforeEach(() => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      map.addHashIndex(categoryAttr);
    });

    it('should update index on add', () => {
      const product: Product = { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true };
      map.add('product1', product);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(1);
      expect(results[0].value.name).toBe('Widget');
    });

    it('should update index on remove', () => {
      const product: Product = { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true };
      map.add('product1', product);

      map.remove('product1', product);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(0);
    });

    it('should update index on apply (remote record)', () => {
      const hlc2 = new HLC('node2');
      const record = {
        value: { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true },
        timestamp: hlc2.now(),
        tag: HLC.toString(hlc2.now()),
      };

      map.apply('product1', record);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(1);
    });

    it('should update index on applyTombstone', () => {
      const record = map.add('product1', {
        name: 'Widget',
        category: 'Electronics',
        price: 99.99,
        inStock: true
      });

      map.applyTombstone(record.tag);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(0);
    });
  });

  describe('composite key handling', () => {
    it('should correctly handle keys with colons', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      map.addHashIndex(categoryAttr);

      // Key with colons should still work correctly
      const product: Product = { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true };
      map.add('product:v1:final', product);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const results = map.query(query);

      expect(results).toHaveLength(1);
      expect(results[0].key).toBe('product:v1:final');
    });
  });

  describe('query explanation', () => {
    it('should explain query execution plan', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      map.addHashIndex(categoryAttr);

      const query: Query = { type: 'eq', attribute: 'category', value: 'Electronics' };
      const plan = map.explainQuery(query);

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(true);
      expect(plan.root.type).toBe('index-scan');
    });

    it('should show full scan for unindexed query', () => {
      const query: Query = { type: 'eq', attribute: 'name', value: 'Widget' };
      const plan = map.explainQuery(query);

      expect(plan).toBeDefined();
      expect(plan.usesIndexes).toBe(false);
      expect(plan.root.type).toBe('full-scan');
    });
  });

  describe('statistics', () => {
    it('should return index statistics', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      map.addHashIndex(categoryAttr);

      map.add('product1', { name: 'Widget', category: 'Electronics', price: 99.99, inStock: true });
      map.add('product2', { name: 'Gadget', category: 'Electronics', price: 199.99, inStock: true });
      map.add('product3', { name: 'Blender', category: 'Appliances', price: 49.99, inStock: true });

      const stats = map.getIndexStats();

      expect(stats.size).toBe(1);
      expect(stats.get('category')).toBeDefined();
      expect(stats.get('category')!.distinctValues).toBe(2);
      expect(stats.get('category')!.totalEntries).toBe(3);
    });

    it('should return registry statistics', () => {
      const categoryAttr = simpleAttribute<Product, string>('category', (p) => p.category);
      const priceAttr = simpleAttribute<Product, number>('price', (p) => p.price);
      map.addHashIndex(categoryAttr);
      map.addNavigableIndex(priceAttr);

      const stats = map.getIndexRegistryStats();

      expect(stats.totalIndexes).toBe(2);
      expect(stats.indexedAttributes).toBe(2);
    });
  });

  // ==================== Full-Text Search (Phase 11) ====================
  describe('Full-Text Search', () => {
    interface Article {
      title: string;
      body: string;
      author?: string;
    }

    let articleMap: IndexedORMap<string, Article>;

    beforeEach(() => {
      articleMap = new IndexedORMap<string, Article>(hlc);
    });

    describe('enableFullTextSearch', () => {
      it('should enable full-text search on specified fields', () => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });

        expect(articleMap.isFullTextSearchEnabled()).toBe(true);
        expect(articleMap.getFullTextIndex()).not.toBeNull();
      });

      it('should build index from existing data', () => {
        articleMap.add('article1', { title: 'Hello World', body: 'This is a test' });
        articleMap.add('article2', { title: 'Goodbye', body: 'Another document' });

        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });

        const results = articleMap.search('hello');
        expect(results).toHaveLength(1);
        expect(results[0].value.title).toBe('Hello World');
      });

      it('should support custom tokenizer options', () => {
        articleMap.enableFullTextSearch({
          fields: ['title'],
          tokenizer: { minLength: 4 },
        });

        articleMap.add('article1', { title: 'The big brown fox', body: 'content' });

        // 'big' has 3 chars, should be filtered
        expect(articleMap.search('big')).toHaveLength(0);
        expect(articleMap.search('brown')).toHaveLength(1);
      });

      it('should support custom BM25 parameters', () => {
        articleMap.enableFullTextSearch({
          fields: ['title'],
          bm25: { k1: 1.5, b: 0.5 },
        });

        articleMap.add('article1', { title: 'Test document', body: 'content' });
        const results = articleMap.search('test');

        expect(results).toHaveLength(1);
      });
    });

    describe('search method', () => {
      beforeEach(() => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
      });

      it('should return ranked results', () => {
        articleMap.add('article1', { title: 'Apple', body: 'fruit' });
        articleMap.add('article2', { title: 'Apple pie', body: 'Apple is great' });
        articleMap.add('article3', { title: 'Banana', body: 'yellow' });

        const results = articleMap.search('apple');

        expect(results).toHaveLength(2);
        expect(results[0].score).toBeGreaterThanOrEqual(results[1].score);
      });

      it('should include key, tag, value, score, and matchedTerms', () => {
        articleMap.add('article1', { title: 'Quick brown fox', body: 'Jumps over' });

        const results = articleMap.search('quick fox');

        expect(results).toHaveLength(1);
        expect(results[0].key).toBe('article1');
        expect(results[0].tag).toBeDefined();
        expect(results[0].value.title).toBe('Quick brown fox');
        expect(results[0].score).toBeGreaterThan(0);
        expect(results[0].matchedTerms).toContain('quick');
        expect(results[0].matchedTerms).toContain('fox');
      });

      it('should throw if full-text search not enabled', () => {
        const newMap = new IndexedORMap<string, Article>(hlc);

        expect(() => newMap.search('test')).toThrow('Full-text search is not enabled');
      });

      it('should return empty results for stopwords-only query', () => {
        articleMap.add('article1', { title: 'Test document', body: 'content' });

        const results = articleMap.search('the a an');
        expect(results).toHaveLength(0);
      });

      it('should return empty results for empty query', () => {
        articleMap.add('article1', { title: 'Test document', body: 'content' });

        const results = articleMap.search('');
        expect(results).toHaveLength(0);
      });
    });

    describe('search options', () => {
      beforeEach(() => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
      });

      it('should limit results', () => {
        for (let i = 0; i < 20; i++) {
          articleMap.add(`article${i}`, { title: 'common term', body: 'content' });
        }

        const results = articleMap.search('common', { limit: 5 });
        expect(results).toHaveLength(5);
      });

      it('should filter by minScore', () => {
        articleMap.add('article1', { title: 'exact match term', body: 'content' });
        articleMap.add('article2', { title: 'term appears once', body: 'content' });

        const results = articleMap.search('exact match term', { minScore: 0.5 });
        expect(results.every((r) => r.score >= 0.5)).toBe(true);
      });

      it('should apply field boost', () => {
        articleMap.add('article1', { title: 'keyword special', body: 'content text' });
        articleMap.add('article2', { title: 'content text', body: 'keyword special' });
        articleMap.add('filler1', { title: 'filler text', body: 'filler text' });
        articleMap.add('filler2', { title: 'filler text', body: 'filler text' });

        const results = articleMap.search('keyword', { boost: { title: 2.0 } });

        expect(results).toHaveLength(2);
        expect(results[0].value.title).toBe('keyword special');
      });
    });

    describe('incremental updates', () => {
      beforeEach(() => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
      });

      it('should update index on add', () => {
        articleMap.add('article1', { title: 'Original', body: 'content' });
        expect(articleMap.search('original')).toHaveLength(1);
      });

      it('should update index on remove', () => {
        const article = { title: 'Test document', body: 'content' };
        articleMap.add('article1', article);
        expect(articleMap.search('test')).toHaveLength(1);

        articleMap.remove('article1', article);
        expect(articleMap.search('test')).toHaveLength(0);
      });

      it('should update index on apply', () => {
        const hlc2 = new HLC('node2');
        const map2 = new IndexedORMap<string, Article>(hlc2);
        const record = map2.add('article1', { title: 'Remote document', body: 'content' });

        articleMap.apply('article1', record);
        expect(articleMap.search('remote')).toHaveLength(1);
      });

      it('should update index on applyTombstone', () => {
        const record = articleMap.add('article1', { title: 'Test document', body: 'content' });
        expect(articleMap.search('test')).toHaveLength(1);

        articleMap.applyTombstone(record.tag);
        expect(articleMap.search('test')).toHaveLength(0);
      });

      it('should clear full-text index on clear', () => {
        articleMap.add('article1', { title: 'Test document', body: 'content' });
        expect(articleMap.search('test')).toHaveLength(1);

        articleMap.clear();
        expect(articleMap.search('test')).toHaveLength(0);
      });
    });

    describe('disableFullTextSearch', () => {
      it('should disable full-text search', () => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
        articleMap.add('article1', { title: 'Test', body: 'content' });

        articleMap.disableFullTextSearch();

        expect(articleMap.isFullTextSearchEnabled()).toBe(false);
        expect(articleMap.getFullTextIndex()).toBeNull();
      });

      it('should throw when searching after disable', () => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
        articleMap.disableFullTextSearch();

        expect(() => articleMap.search('test')).toThrow('Full-text search is not enabled');
      });
    });

    describe('edge cases', () => {
      beforeEach(() => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });
      });

      it('should handle documents with missing fields', () => {
        articleMap.add('article1', { title: 'Only title' } as Article);
        expect(articleMap.search('title')).toHaveLength(1);
      });

      it('should handle unicode text', () => {
        articleMap.add('article1', { title: 'Привет мир', body: 'content' });
        expect(articleMap.search('привет')).toHaveLength(1);
      });

      it('should handle very long text', () => {
        const longText = 'word '.repeat(10000);
        expect(() => articleMap.add('article1', { title: 'Test', body: longText })).not.toThrow();
        expect(articleMap.search('word')).toHaveLength(1);
      });

      it('should handle special characters', () => {
        articleMap.add('article1', { title: 'Test <script>alert("xss")</script>', body: 'content' });
        expect(articleMap.search('test')).toHaveLength(1);
        expect(articleMap.search('script')).toHaveLength(1);
      });

      it('should handle rapid updates', () => {
        for (let i = 0; i < 100; i++) {
          articleMap.add('article1', { title: `Version ${i}`, body: 'content' });
        }

        expect(articleMap.search('version')).toHaveLength(100);
      });
    });

    describe('performance', () => {
      it('should build index efficiently (1K docs < 100ms)', () => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });

        const start = performance.now();
        for (let i = 0; i < 1000; i++) {
          articleMap.add(`article${i}`, {
            title: `Document ${i} about topic`,
            body: `This is the body of document ${i} with some content`,
          });
        }
        const duration = performance.now() - start;

        expect(duration).toBeLessThan(200); // 200ms to account for ORMap overhead
      });

      it('should search efficiently (1K docs < 10ms)', () => {
        articleMap.enableFullTextSearch({ fields: ['title', 'body'] });

        for (let i = 0; i < 1000; i++) {
          articleMap.add(`article${i}`, {
            title: `Document ${i}`,
            body: `Content with common terms`,
          });
        }

        const start = performance.now();
        articleMap.search('common terms document');
        const duration = performance.now() - start;

        expect(duration).toBeLessThan(10);
      });
    });
  });
});
