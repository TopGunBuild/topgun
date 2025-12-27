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
});
